use crate::config::{Adapters, Governance, RuntimeConfig};
use crate::effect_dispatch::EffectWork;
use crate::runtime::{EffectPreparation, MaestriaRuntime};
use crate::runtime_transition::TransitionBarriers;
use maestria_domain::{DomainEventEnvelope, DomainInput, KernelState, MaestriaEffect};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::{RwLock, mpsc, oneshot};

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
mod pipeline;

pub(crate) struct ApplicationOutcome {
    pub(crate) events: Vec<DomainEventEnvelope>,
    pub(crate) effects_admitted: usize,
}

struct StagedInput {
    completed_run_id: Option<maestria_domain::HarnessRunId>,
    approval_continuation: Option<maestria_domain::ModelAgentProposalRequest>,
    should_resume_approval: bool,
    barriers: TransitionBarriers,
    pre_event_count: usize,
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
        let next_command_id = Arc::new(AtomicU64::new(Self::seed_next_command_id(&state)));
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
                degraded_vector_artifacts: Arc::new(Mutex::new(BTreeMap::new())),
                full_text_locks: Arc::new(Mutex::new(BTreeMap::new())),
                pending_effect_batches: Arc::new(AtomicUsize::new(0)),
                in_flight_effects: Arc::new(AtomicUsize::new(0)),
                executor_quiescent: Arc::new(AtomicBool::new(true)),
                admission_open: Arc::new(AtomicBool::new(true)),
                pending_applications: Mutex::new(BTreeMap::new()),
                pending_notebook_drafts: Mutex::new(BTreeMap::new()),
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
    /// executor has observed the selected shutdown policy and preserves
    /// recovery or task-join failures for the lifecycle owner.
    pub async fn run(
        mut self,
        input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) -> Result<(), crate::runtime::RuntimeRunError> {
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
                return Err(crate::runtime::RuntimeRunError::RecoveryPlanning {
                    reason: error.to_string(),
                });
            }
        };
        let Some(command_rx) = self.command_rx.take() else {
            tracing::error!("runtime command receiver missing");
            return Err(crate::runtime::RuntimeRunError::CommandReceiverUnavailable);
        };

        let queue_recovery =
            Self::queue_model_agent_recovery(recovery, &shutdown_token, &effect_tx);
        let mut input_rx = input_rx;
        let run_inputs =
            self.run_input_loop(&mut input_rx, command_rx, &effect_tx, &shutdown_token);
        tokio::join!(queue_recovery, run_inputs);
        if self.config.drain_effects_on_shutdown {
            // Graceful mode: keep servicing the domain-input channel while
            // in-flight effects finish, so their completion inputs are
            // persisted instead of racing a closed channel.
            // Exits when the executor reports quiescence (no queued batch,
            // no running effect) or when `shutdown_drain_grace` elapses;
            // stragglers degrade through the deferred-delivery path.
            self.drain_effect_completions(&mut input_rx, &effect_tx, &shutdown_token)
                .await;
        }
        // The drain is over: late submissions must fail fast instead of
        // feeding batches to a runtime that is tearing down.
        self.admission_open
            .store(false, std::sync::atomic::Ordering::Relaxed);
        drop(effect_tx);
        if !self.config.drain_effects_on_shutdown {
            effect_shutdown.cancel();
        }
        shutdown_token.cancel();
        effect_executor.await.map_err(|error| {
            crate::runtime::RuntimeRunError::EffectExecutorJoin {
                reason: error.to_string(),
            }
        })?;
        if let Some(flush_projections) = self.config.flush_projections.as_ref() {
            flush_projections();
        }
        Ok(())
    }

    async fn run_input_loop(
        &self,
        input_rx: &mut mpsc::Receiver<DomainInput>,
        mut command_rx: mpsc::Receiver<(DomainInput, crate::runtime::RuntimeCommand)>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) {
        loop {
            let incoming = tokio::select! {
                () = shutdown_token.cancelled() => break,
                input = input_rx.recv() => input.map(|input| (input, None)),
                command = command_rx.recv() => {
                    command.map(|(input, command)| (input, Some(command)))
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

    /// Service completion deliveries until the effect executor is
    /// quiescent (nothing queued, nothing running) or the grace elapses.
    ///
    /// The double `try_recv` after observing quiescence closes the race
    /// where a completion input arrives between the queue scan and the
    /// quiescence check: if one slipped in, the loop continues and waits
    /// for the effects it admits.
    async fn drain_effect_completions(
        &self,
        input_rx: &mut mpsc::Receiver<DomainInput>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) {
        // The tokio clock (not the wall clock) drives the grace so test
        // time control and the async runtime agree on deadlines.
        let deadline = tokio::time::Instant::now() + self.config.shutdown_drain_grace;
        loop {
            while let Ok(input) = input_rx.try_recv() {
                if !self
                    .process_input(input, None, effect_tx, shutdown_token)
                    .await
                {
                    return;
                }
            }
            if self.executor_quiescent.load(Ordering::Relaxed) && input_rx.try_recv().is_err() {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    "shutdown drain grace elapsed; deferring remaining completion deliveries"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
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
