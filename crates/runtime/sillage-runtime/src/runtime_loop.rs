use crate::effect_dispatch::EffectWork;
use crate::runtime::{EffectPreparation, SillageRuntime};
use crate::runtime_transition::TransitionBarriers;
use sillage_domain::{DomainEventEnvelope, DomainInput, SillageEffect};
use tokio::sync::{mpsc, oneshot};

/// Best-effort delivery of a command reply (R24: send failures are not
/// silently discarded). A disconnected receiver is expected when the
/// submitter cancelled or the runtime is shutting down, so it is logged at
/// debug level instead of being dropped with `let _ =`.
pub(super) fn deliver_reply<T>(reply: oneshot::Sender<T>, result: T) {
    if reply.send(result).is_err() {
        tracing::debug!("command reply receiver disconnected; result dropped");
    }
}

mod continuation;
mod lifecycle;
mod pipeline;

pub(crate) struct ApplicationOutcome {
    pub(crate) events: Vec<DomainEventEnvelope>,
    pub(crate) effects_admitted: usize,
}

struct StagedInput {
    completed_run_id: Option<sillage_domain::HarnessRunId>,
    approval_continuation: Option<sillage_domain::ModelAgentProposalRequest>,
    should_resume_approval: bool,
    barriers: TransitionBarriers,
    pre_event_count: usize,
}
enum EffectBatchPreparationError {
    Admission,
    Preparation(String),
}

impl SillageRuntime {
    async fn prepare_effect_work(
        &self,
        effects: &[SillageEffect],
        prepare_before_reply: bool,
    ) -> Result<Option<Vec<EffectWork>>, String> {
        if !prepare_before_reply {
            return Ok(None);
        }
        let context = self.effect_execution_context();
        let mut prepared = Vec::with_capacity(effects.len());
        for effect in effects.iter().cloned() {
            let effect = context
                .prepare_effect_before_reply(effect)
                .await
                .map_err(|error| error.to_string())?;
            prepared.push(EffectWork::Prepared(effect));
        }
        Ok(Some(prepared))
    }
    async fn prepare_effect_batch<'a>(
        &self,
        effects: &[SillageEffect],
        prepare_before_reply: bool,
        effect_tx: &'a mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> Result<
        (
            Option<mpsc::Permit<'a, crate::effect_dispatch::EffectBatch>>,
            Option<Vec<EffectWork>>,
        ),
        EffectBatchPreparationError,
    > {
        let permit = if effects.is_empty() {
            None
        } else {
            Some(
                self.reserve_effect_batch(effect_tx, shutdown_token)
                    .await
                    .map_err(|_| EffectBatchPreparationError::Admission)?,
            )
        };
        let prepared = self
            .prepare_effect_work(effects, prepare_before_reply)
            .await
            .map_err(EffectBatchPreparationError::Preparation)?;
        Ok((permit, prepared))
    }

    pub(crate) async fn process_input(
        &self,
        input: DomainInput,
        mut command: Option<crate::runtime::RuntimeCommand>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let input = Self::correlate_proposal(input, command.as_ref());
        if command.is_none() {
            let correlation_id = match &input {
                DomainInput::NotebookDraftBlobStored(stored) => stored.correlation_id,
                _ => None,
            };
            command = self.take_notebook_draft_command(correlation_id);
        }
        let completed_run_id = Self::completed_run_id(&input);
        if let DomainInput::NotebookDraftBlobStoreFailed(failure) = &input {
            if let Some(command) = command.take() {
                Self::reply_preparation_error(Some(command), failure.reason.clone());
            }
            return true;
        }
        let approval_continuation = match self.resolve_approval_continuation(&input, &mut command) {
            Ok(continuation) => continuation,
            Err(()) => return true,
        };
        if self
            .reject_out_of_boundary_input(&input, &mut command)
            .await
        {
            return true;
        }
        let harness_feedback = Self::harness_feedback(&input);
        let approval_barrier = Self::approval_barrier(&input, command.as_ref());
        let Some(staged) = self
            .stage_correlated_input(input.clone(), approval_continuation.is_some(), &mut command)
            .await
        else {
            return true;
        };
        let output = staged.output;
        let should_resume_approval = staged.should_resume_approval;
        let mut effects = output.effects;
        Self::assign_notebook_draft_correlation(
            &input,
            &mut effects,
            command.as_ref().map(|command| command.correlation_id),
        );
        let prepare_before_reply = command
            .as_ref()
            .is_some_and(|command| command.effect_preparation == EffectPreparation::BeforeReply);
        let Some((permit, prepared)) = self
            .admit_and_prepare_effects(
                &effects,
                prepare_before_reply,
                effect_tx,
                shutdown_token,
                &mut command,
            )
            .await
        else {
            return !shutdown_token.is_cancelled();
        };
        let mut outcome = ApplicationOutcome {
            effects_admitted: effects.len(),
            events: output.events.clone(),
        };
        let barriers = Self::transition_barriers(
            &outcome.events,
            &effects,
            approval_barrier,
            prepare_before_reply,
        );
        self.commit_staged_input(harness_feedback, &effects);
        if !self
            .dispatch_admitted_effects(permit, prepared, effects, &mut command, shutdown_token)
            .await
        {
            return false;
        }
        if matches!(input, DomainInput::SaveNotebookDraftRequested(_))
            && let Some(correlation_id) = command.as_ref().map(|command| command.correlation_id)
            && let Some(command) = command.take()
        {
            if let Ok(mut pending) = self.pending_notebook_drafts.lock() {
                pending.insert(correlation_id, command);
            } else {
                tracing::error!("notebook draft pending-command lock poisoned");
                return false;
            }
        }
        self.await_persistence_and_finalize(
            StagedInput {
                completed_run_id,
                approval_continuation,
                should_resume_approval,
                barriers,
                pre_event_count: staged.pre_event_count,
            },
            &mut command,
            &mut outcome,
            effect_tx,
            shutdown_token,
        )
        .await
    }
}
