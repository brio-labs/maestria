//! Input-processing phases for the runtime loop.
//!
//! `process_input` in `super` correlates, stages, admits, dispatches, and
//! finalizes one domain input. Each phase is a typed method here so the
//! input pipeline owns one concept per function (R20) and the loop module
//! stays within its size budget.

use super::{ApplicationOutcome, EffectBatchPreparationError, StagedInput};
use crate::effect_dispatch::EffectWork;
use crate::runtime::{DomainApplicationResult, MaestriaRuntime};
use maestria_domain::{DomainError, DomainInput, KernelOutput, KernelState, MaestriaEffect};
use tokio::sync::mpsc;

impl MaestriaRuntime {
    /// Resolve the stored approval continuation for a correlated input.
    ///
    /// Returns `Ok(None)` when the input carries no continuation, and `Err(())`
    /// when the stored record claims to carry one but is corrupt: the
    /// preparation error has already been replied and the caller must stop
    /// processing the input.
    pub(super) fn resolve_approval_continuation(
        &self,
        input: &DomainInput,
        command: &mut Option<crate::runtime::RuntimeCommand>,
    ) -> Result<Option<maestria_domain::ModelAgentProposalRequest>, ()> {
        match self.approval_continuation(input) {
            Ok(continuation) => Ok(continuation),
            // Reject the input rather than discarding the failure and
            // continuing without the pending proposal.
            Err(reason) => {
                Self::reply_preparation_error(command.take(), reason);
                Err(())
            }
        }
    }

    /// Reject inputs that fail boundary validation, replying the invariant
    /// violation. Returns `true` when the input was rejected.
    pub(super) async fn reject_out_of_boundary_input(
        &self,
        input: &DomainInput,
        command: &mut Option<crate::runtime::RuntimeCommand>,
    ) -> bool {
        if let Some(detail) = self.boundary_error(input).await {
            Self::reply_domain_error(
                command.take(),
                DomainError::InternalInvariantViolation { detail },
            );
            return true;
        }
        false
    }

    /// Stage the correlated input against a candidate kernel state, replying
    /// the domain error when staging fails. Returns `None` when an error has
    /// already been replied.
    pub(super) async fn stage_correlated_input(
        &self,
        input: DomainInput,
        resume_approval: bool,
        command: &mut Option<crate::runtime::RuntimeCommand>,
    ) -> Option<(KernelState, KernelOutput, bool, KernelState)> {
        match self.stage_input(input, resume_approval).await {
            Ok(staged) => Some(staged),
            Err(error) => {
                Self::reply_domain_error(command.take(), error);
                None
            }
        }
    }

    /// Admit and prepare the staged effects, replying admission or
    /// preparation errors. Returns `None` when an error has already been
    /// replied and the caller must return `!shutdown_token.is_cancelled()`.
    pub(super) async fn admit_and_prepare_effects<'a>(
        &self,
        effects: &[MaestriaEffect],
        prepare_before_reply: bool,
        effect_tx: &'a mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
        command: &mut Option<crate::runtime::RuntimeCommand>,
    ) -> Option<(
        Option<mpsc::Permit<'a, crate::effect_dispatch::EffectBatch>>,
        Option<Vec<EffectWork>>,
    )> {
        match self
            .prepare_effect_batch(effects, prepare_before_reply, effect_tx, shutdown_token)
            .await
        {
            Ok(prepared) => Some(prepared),
            Err(EffectBatchPreparationError::Admission) => {
                Self::reply_admission_error(command.take());
                None
            }
            Err(EffectBatchPreparationError::Preparation(error)) => {
                tracing::warn!(%error, "effect rejected before correlated reply");
                Self::reply_preparation_error(command.take(), error);
                None
            }
        }
    }

    /// Swap the candidate kernel state into the runtime and register any
    /// harness feedback produced by the staged input.
    pub(super) async fn commit_staged_input(
        &self,
        candidate: KernelState,
        harness_feedback: Option<(maestria_domain::HarnessRunId, u64)>,
        effects: &[MaestriaEffect],
    ) {
        let mut state = self.state.write().await;
        *state = candidate;
        self.register_harness_feedback(harness_feedback, effects);
    }

    /// Dispatch the admitted effects to the executor, replying the admission
    /// error and cancelling the runtime when the dispatch channel is closed.
    pub(super) async fn dispatch_admitted_effects(
        &self,
        permit: Option<mpsc::Permit<'_, crate::effect_dispatch::EffectBatch>>,
        prepared: Option<Vec<EffectWork>>,
        effects: Vec<MaestriaEffect>,
        command: &mut Option<crate::runtime::RuntimeCommand>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let effect_batch = match prepared {
            Some(prepared) => prepared,
            None => effects.into_iter().map(EffectWork::Pending).collect(),
        };
        if let Some(permit) = permit
            && self.send_reserved_effects(permit, effect_batch).is_err()
        {
            Self::reply_admission_error(command.take());
            shutdown_token.cancel();
            return false;
        }
        true
    }

    /// Wait for the transition persistence barriers, then finalize the
    /// application: merge or defer the approval continuation, complete
    /// deferred applications, reply, and await the validation barrier.
    pub(super) async fn await_persistence_and_finalize(
        &self,
        staged: StagedInput,
        command: &mut Option<crate::runtime::RuntimeCommand>,
        outcome: &mut ApplicationOutcome,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        if !self
            .wait_transition_barriers(&staged.barriers, command.is_some(), shutdown_token)
            .await
        {
            let mut state = self.state.write().await;
            *state = staged.previous_state;
            Self::reply_persistence_error(command.take());
            return true;
        }
        self.finalize_application(staged, command, outcome, effect_tx, shutdown_token)
            .await
    }

    /// Finalize a processed input: merge or defer the approval continuation,
    /// complete deferred applications, reply, and await the validation barrier.
    pub(super) async fn finalize_application(
        &self,
        staged: StagedInput,
        command: &mut Option<crate::runtime::RuntimeCommand>,
        outcome: &mut ApplicationOutcome,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let deferred_run_id = match self
            .merge_inline_approval_continuation(
                staged.approval_continuation,
                staged.should_resume_approval,
                command,
                outcome,
                effect_tx,
                shutdown_token,
            )
            .await
        {
            Ok(run_id) => run_id,
            Err(keep_running) => return keep_running,
        };
        if let Some(run_id) = staged.completed_run_id {
            self.complete_pending_application(run_id, outcome);
        }
        if let Some(run_id) = deferred_run_id
            && let Some(command) = command.take()
        {
            let outcome = std::mem::replace(
                outcome,
                ApplicationOutcome {
                    events: Vec::new(),
                    effects_admitted: 0,
                },
            );
            return self.defer_application(run_id, command, outcome);
        }
        if let Some(command) = command.take() {
            super::deliver_reply(
                command.reply,
                Ok(DomainApplicationResult {
                    correlation_id: command.correlation_id,
                    events: std::mem::take(&mut outcome.events),
                    effects_admitted: outcome.effects_admitted,
                }),
            );
        }
        self.finish_validation_barrier(staged.barriers.validation_report_id, shutdown_token)
            .await
    }
}
