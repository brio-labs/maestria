use crate::config::{Adapters, Governance, RuntimeConfig};
use crate::effect_dispatch::EffectWork;
use crate::runtime::{EffectPreparation, MaestriaRuntime};
use crate::runtime_transition::TransitionBarriers;
use maestria_domain::{DomainEventEnvelope, DomainInput, KernelState, MaestriaEffect};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, atomic::AtomicU64};
use tokio::sync::{RwLock, mpsc};

mod continuation;
mod pipeline;

pub(crate) struct ApplicationOutcome {
    pub(crate) events: Vec<DomainEventEnvelope>,
    pub(crate) effects_admitted: usize,
}

/// Per-input data produced before staging that finalization needs.
struct StagedInput {
    completed_run_id: Option<maestria_domain::HarnessRunId>,
    approval_continuation: Option<maestria_domain::ModelAgentProposalRequest>,
    should_resume_approval: bool,
    barriers: TransitionBarriers,
}
enum EffectBatchPreparationError {
    Admission,
    Preparation(String),
}

impl MaestriaRuntime {
    pub fn new(
        mut config: RuntimeConfig,
        state: KernelState,
        adapters: Adapters,
        governance: Governance,
    ) -> (Self, mpsc::Receiver<DomainInput>) {
        config.max_concurrent_effects = config.max_concurrent_effects.max(1);
        config.input_buffer_size = config.input_buffer_size.max(1);
        let (input_tx, input_rx) = mpsc::channel(config.input_buffer_size);
        let (command_tx, command_rx) = mpsc::channel(config.input_buffer_size);
        let next_command_id = Arc::new(AtomicU64::new(1));
        let next_validation_report_id =
            Arc::new(AtomicU64::new(Self::seed_next_validation_report_id(&state)));
        (
            Self {
                config,
                state: Arc::new(RwLock::new(state)),
                adapters: Arc::new(adapters),
                governance: Arc::new(governance),
                input_tx,
                command_tx,
                command_rx: Some(command_rx),
                next_command_id,
                journal_recovery_claims: Arc::new(Mutex::new(BTreeSet::new())),
                feedback_acks: Arc::new(Mutex::new(BTreeMap::new())),
                pending_applications: Mutex::new(BTreeMap::new()),
                next_validation_report_id,
                #[cfg(test)]
                test_pre_failed_effect_task: false,
            },
            input_rx,
        )
    }

    /// Runs the domain-input loop until the shutdown token is cancelled or
    /// the input channel closes.
    ///
    /// Cancellation stops accepting new inputs. By default, in-flight effects
    /// are cancelled; call [`Self::with_graceful_shutdown`] before `run` to
    /// drain already-started effects. The method returns after the effect
    /// executor has observed the selected shutdown policy.
    pub async fn run(
        mut self,
        input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        let (effect_tx, effect_rx) =
            mpsc::channel::<crate::effect_dispatch::EffectBatch>(self.config.input_buffer_size);
        let effect_shutdown = tokio_util::sync::CancellationToken::new();
        let effect_executor =
            self.spawn_effect_executor(effect_rx, effect_shutdown.clone(), shutdown_token.clone());
        let recovery_snapshot = self.state.read().await.clone();
        let recovery = match self.plan_model_agent_recovery(&recovery_snapshot) {
            Ok(recovery) => recovery,
            Err(error) => {
                // Recovery planning is authoritative for in-flight model-agent
                // effects. Refuse to start instead of silently dropping them.
                tracing::error!(
                    %error,
                    "model-agent recovery planning failed; runtime will not start"
                );
                effect_shutdown.cancel();
                shutdown_token.cancel();
                return;
            }
        };
        let Some(command_rx) = self.command_rx.take() else {
            tracing::error!("runtime command receiver missing");
            return;
        };

        let queue_recovery =
            Self::queue_model_agent_recovery(recovery, &shutdown_token, &effect_tx);
        let run_inputs = self.run_input_loop(input_rx, command_rx, &effect_tx, &shutdown_token);
        tokio::join!(queue_recovery, run_inputs);
        drop(effect_tx);
        if !self.config.drain_effects_on_shutdown {
            effect_shutdown.cancel();
        }
        shutdown_token.cancel();
        if let Err(error) = effect_executor.await {
            tracing::error!(%error, "effect executor task failed");
        }
    }

    async fn run_input_loop(
        &self,
        mut input_rx: mpsc::Receiver<DomainInput>,
        mut command_rx: mpsc::Receiver<crate::runtime::RuntimeCommand>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) {
        loop {
            let incoming = tokio::select! {
                () = shutdown_token.cancelled() => break,
                input = input_rx.recv() => input.map(|input| (input, None)),
                command = command_rx.recv() => {
                    command.map(|command| (command.input.clone(), Some(command)))
                },
            };
            let Some((input, command)) = incoming else {
                break;
            };
            if !self
                .process_input(input, command, effect_tx, shutdown_token)
                .await
            {
                break;
            }
        }
    }

    async fn prepare_effect_work(
        &self,
        effects: &[MaestriaEffect],
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
        effects: &[MaestriaEffect],
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
        let completed_run_id = Self::completed_run_id(&input);
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
        let Some((candidate, output, should_resume_approval)) = self
            .stage_correlated_input(input, approval_continuation.is_some(), &mut command)
            .await
        else {
            return true;
        };
        let effects = output.effects;
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
        let barriers = Self::transition_barriers(&outcome.events, &effects, approval_barrier);
        self.commit_staged_input(candidate, harness_feedback, &effects)
            .await;
        if !self
            .dispatch_admitted_effects(permit, prepared, effects, &mut command, shutdown_token)
            .await
        {
            return false;
        }
        self.await_persistence_and_finalize(
            StagedInput {
                completed_run_id,
                approval_continuation,
                should_resume_approval,
                barriers,
            },
            &mut command,
            &mut outcome,
            effect_tx,
            shutdown_token,
        )
        .await
    }
}
