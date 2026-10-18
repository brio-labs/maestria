use super::*;
use crate::config::{Adapters, Governance, RuntimeConfig};
use crate::runtime::RuntimeCommand;
use sillage_domain::KernelState;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug)]
enum ShutdownDrainOutcome {
    NotRequested,
    Settled,
    GraceElapsed,
    Stopped,
}

enum InputProcessOutcome {
    Completed(bool),
    GraceElapsed,
    Shutdown,
}

type RuntimeLoopWait =
    ControlFlow<ShutdownDrainOutcome, Option<(DomainInput, Option<RuntimeCommand>)>>;

#[derive(Default)]
struct InputChannelClosure {
    input: bool,
    command: bool,
    feedback: bool,
}

struct RuntimeLoopState<'a> {
    input_rx: &'a mut mpsc::Receiver<DomainInput>,
    command_rx: &'a mut mpsc::Receiver<(DomainInput, RuntimeCommand)>,
    feedback_rx: &'a mut mpsc::Receiver<DomainInput>,
    effect_tx: &'a mpsc::Sender<crate::effect_dispatch::EffectBatch>,
    shutdown_token: &'a tokio_util::sync::CancellationToken,
    drain_token: &'a tokio_util::sync::CancellationToken,
    deadline: Option<tokio::time::Instant>,
    closure: InputChannelClosure,
}
impl SillageRuntime {
    pub fn new(
        mut config: RuntimeConfig,
        state: KernelState,
        adapters: Adapters,
        governance: Governance,
    ) -> (Self, mpsc::Receiver<DomainInput>) {
        config.max_concurrent_effects = config.max_concurrent_effects.max(1);
        config.input_buffer_size = config.input_buffer_size.max(1);
        let (input_tx, input_rx) = mpsc::channel(config.input_buffer_size);
        let (feedback_tx, feedback_rx) = mpsc::channel(config.input_buffer_size);
        let (command_tx, command_rx) = mpsc::channel(config.input_buffer_size);
        let next_command_id = Arc::new(AtomicU64::new(Self::seed_next_command_id(&state)));
        (
            Self {
                config,
                state: Arc::new(RwLock::new(state)),
                adapters: Arc::new(adapters),
                governance: Arc::new(governance),
                input_tx,
                feedback_tx,
                feedback_rx: Some(feedback_rx),
                command_tx,
                command_rx: Some(command_rx),
                next_command_id,
                journal_recovery_claims: Arc::new(Mutex::new(BTreeSet::new())),
                feedback_acks: Arc::new(Mutex::new(BTreeMap::new())),
                degraded_vector_artifacts: Arc::new(Mutex::new(BTreeMap::new())),
                full_text_batch_lock: Arc::new(tokio::sync::Mutex::new(())),
                pending_effect_batches: Arc::new(AtomicUsize::new(0)),
                in_flight_effects: Arc::new(AtomicUsize::new(0)),
                executor_quiescence_notify: Arc::new(tokio::sync::Notify::new()),
                admission_open: Arc::new(AtomicBool::new(true)),
                pending_applications: Mutex::new(BTreeMap::new()),
                pending_notebook_drafts: Mutex::new(BTreeMap::new()),
                #[cfg(test)]
                test_pre_failed_effect_task: false,
                #[cfg(test)]
                test_settlement_check_hook: None,
            },
            input_rx,
        )
    }

    /// Runs until cancellation, then settles accepted work and transitive
    /// effect feedback within the configured single shutdown grace.
    pub async fn run(
        mut self,
        input_rx: mpsc::Receiver<DomainInput>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) -> Result<(), crate::runtime::RuntimeRunError> {
        let recovery_snapshot = self.state.read().await.clone();
        let recovery = self
            .plan_model_agent_recovery(&recovery_snapshot)
            .map_err(|error| crate::runtime::RuntimeRunError::RecoveryPlanning {
                reason: error.to_string(),
            })?;
        let Some(mut command_rx) = self.command_rx.take() else {
            return Err(crate::runtime::RuntimeRunError::CommandReceiverUnavailable);
        };
        let Some(mut feedback_rx) = self.feedback_rx.take() else {
            return Err(crate::runtime::RuntimeRunError::CommandReceiverUnavailable);
        };
        let (effect_tx, effect_rx) =
            mpsc::channel::<crate::effect_dispatch::EffectBatch>(self.config.input_buffer_size);
        let effect_shutdown = tokio_util::sync::CancellationToken::new();
        let drain_token = tokio_util::sync::CancellationToken::new();
        let effect_executor =
            self.spawn_effect_executor(effect_rx, effect_shutdown.clone(), shutdown_token.clone());
        let recovery_token = if self.config.drain_effects_on_shutdown {
            &drain_token
        } else {
            &shutdown_token
        };
        let recovery_queued = self
            .queue_model_agent_recovery(recovery, recovery_token, &effect_tx)
            .await;
        let mut input_rx = input_rx;
        let drain_outcome = if !recovery_queued {
            if self.config.drain_effects_on_shutdown {
                ShutdownDrainOutcome::Stopped
            } else {
                ShutdownDrainOutcome::NotRequested
            }
        } else {
            self.run_input_loop(
                &mut input_rx,
                &mut command_rx,
                &mut feedback_rx,
                &effect_tx,
                &shutdown_token,
                &drain_token,
            )
            .await
        };
        input_rx.close();
        command_rx.close();
        feedback_rx.close();
        self.admission_open.store(false, Ordering::Release);
        drop(effect_tx);
        if !self.config.drain_effects_on_shutdown
            || !matches!(drain_outcome, ShutdownDrainOutcome::Settled)
        {
            effect_shutdown.cancel();
        }
        shutdown_token.cancel();
        match effect_executor.await {
            Err(source) => {
                return Err(crate::runtime::RuntimeRunError::EffectExecutorJoin { source });
            }
            Ok(Err(crate::effect_executor_shutdown::EffectExecutorFailure::TaskJoin(source))) => {
                return Err(crate::runtime::RuntimeRunError::EffectTaskJoin { source });
            }
            Ok(Err(crate::effect_executor_shutdown::EffectExecutorFailure::Effect(source))) => {
                return Err(crate::runtime::RuntimeRunError::EffectFailure {
                    source: Box::new(source),
                });
            }
            Ok(Ok(())) => {}
        }
        match drain_outcome {
            ShutdownDrainOutcome::GraceElapsed => {
                return Err(crate::runtime::RuntimeRunError::ShutdownDrainGraceElapsed);
            }
            ShutdownDrainOutcome::Stopped => {
                return Err(crate::runtime::RuntimeRunError::ShutdownDrainStopped);
            }
            ShutdownDrainOutcome::NotRequested | ShutdownDrainOutcome::Settled => {}
        }
        if let Some(flush_projections) = self.config.flush_projections.as_ref() {
            flush_projections().map_err(|error| {
                crate::runtime::RuntimeRunError::ProjectionFlush {
                    reason: error.to_string(),
                }
            })?;
        }
        Ok(())
    }

    async fn run_input_loop(
        &self,
        input_rx: &mut mpsc::Receiver<DomainInput>,
        command_rx: &mut mpsc::Receiver<(DomainInput, RuntimeCommand)>,
        feedback_rx: &mut mpsc::Receiver<DomainInput>,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
        shutdown_token: &tokio_util::sync::CancellationToken,
        drain_token: &tokio_util::sync::CancellationToken,
    ) -> ShutdownDrainOutcome {
        let mut state = RuntimeLoopState {
            input_rx,
            command_rx,
            feedback_rx,
            effect_tx,
            shutdown_token,
            drain_token,
            deadline: None,
            closure: InputChannelClosure::default(),
        };
        loop {
            match self.wait_for_input_or_settlement(&mut state).await {
                ControlFlow::Break(outcome) => return outcome,
                ControlFlow::Continue(None) => continue,
                ControlFlow::Continue(Some((input, command))) => {
                    match self
                        .process_input_during_run(input, command, &mut state)
                        .await
                    {
                        InputProcessOutcome::Completed(true) => {}
                        InputProcessOutcome::Completed(false) => {
                            return if self.config.drain_effects_on_shutdown {
                                ShutdownDrainOutcome::Stopped
                            } else {
                                ShutdownDrainOutcome::NotRequested
                            };
                        }
                        InputProcessOutcome::GraceElapsed => {
                            return ShutdownDrainOutcome::GraceElapsed;
                        }
                        InputProcessOutcome::Shutdown => {
                            return ShutdownDrainOutcome::NotRequested;
                        }
                    }
                }
            }
        }
    }

    async fn wait_for_input_or_settlement(
        &self,
        state: &mut RuntimeLoopState<'_>,
    ) -> RuntimeLoopWait {
        let graceful = self.config.drain_effects_on_shutdown;
        let quiescence_changed = self.executor_quiescence_notify.notified();
        tokio::pin!(quiescence_changed);
        quiescence_changed.as_mut().enable();
        if !graceful && state.shutdown_token.is_cancelled() {
            state.input_rx.close();
            state.command_rx.close();
            return ControlFlow::Break(ShutdownDrainOutcome::NotRequested);
        }
        if graceful && state.deadline.is_none() && state.shutdown_token.is_cancelled() {
            state.input_rx.close();
            state.command_rx.close();
            state.deadline = Some(tokio::time::Instant::now() + self.config.shutdown_drain_grace);
        }
        if state.deadline.is_some()
            && state.closure.input
            && state.closure.command
            && self.is_executor_settled(state.feedback_rx)
        {
            return ControlFlow::Break(ShutdownDrainOutcome::Settled);
        }
        let expiry_deadline = state.deadline;
        let expiry = async move {
            if let Some(deadline) = expiry_deadline {
                tokio::time::sleep_until(deadline).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(expiry);
        tokio::select! {
            biased;
            () = state.shutdown_token.cancelled(), if graceful && state.deadline.is_none() => {
                state.input_rx.close();
                state.command_rx.close();
                state.deadline =
                    Some(tokio::time::Instant::now() + self.config.shutdown_drain_grace);
                ControlFlow::Continue(None)
            }
            () = state.shutdown_token.cancelled(), if !graceful => {
                state.input_rx.close();
                state.command_rx.close();
                ControlFlow::Break(ShutdownDrainOutcome::NotRequested)
            }
            () = &mut expiry => {
                state.drain_token.cancel();
                tracing::warn!("shutdown drain grace elapsed; accepted work remains unsettled");
                ControlFlow::Break(ShutdownDrainOutcome::GraceElapsed)
            }
            () = &mut quiescence_changed => ControlFlow::Continue(None),
            input = state.input_rx.recv(), if !state.closure.input => match input {
                Some(input) => ControlFlow::Continue(Some((input, None))),
                None => {
                    state.closure.input = true;
                    ControlFlow::Continue(None)
                }
            },
            command = state.command_rx.recv(), if !state.closure.command => match command {
                Some((input, command)) => {
                    ControlFlow::Continue(Some((input, Some(command))))
                }
                None => {
                    state.closure.command = true;
                    ControlFlow::Continue(None)
                }
            },
            feedback = state.feedback_rx.recv(), if !state.closure.feedback => match feedback {
                Some(input) => ControlFlow::Continue(Some((input, None))),
                None => {
                    state.closure.feedback = true;
                    ControlFlow::Continue(None)
                }
            },
        }
    }

    fn is_executor_settled(&self, feedback_rx: &mpsc::Receiver<DomainInput>) -> bool {
        // Executor children enqueue final feedback before the joiner decrements their count.
        let pending_effect_batches = self.pending_effect_batches.load(Ordering::Acquire);
        let in_flight_effects = self.in_flight_effects.load(Ordering::Acquire);
        #[cfg(test)]
        if let Some(hook) = &self.test_settlement_check_hook {
            hook();
        }
        pending_effect_batches == 0 && in_flight_effects == 0 && feedback_rx.is_empty()
    }

    async fn process_input_during_run(
        &self,
        input: DomainInput,
        command: Option<RuntimeCommand>,
        state: &mut RuntimeLoopState<'_>,
    ) -> InputProcessOutcome {
        let graceful = self.config.drain_effects_on_shutdown;
        let processing_token = if graceful {
            state.drain_token
        } else {
            state.shutdown_token
        };
        let process = self.process_input(input, command, state.effect_tx, processing_token);
        tokio::pin!(process);
        if !graceful {
            return tokio::select! {
                biased;
                () = state.shutdown_token.cancelled() => {
                    state.input_rx.close();
                    state.command_rx.close();
                    InputProcessOutcome::Shutdown
                }
                result = &mut process => InputProcessOutcome::Completed(result),
            };
        }
        if let Some(deadline) = state.deadline {
            return tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => {
                    state.drain_token.cancel();
                    InputProcessOutcome::GraceElapsed
                }
                result = &mut process => InputProcessOutcome::Completed(result),
            };
        }
        tokio::select! {
            biased;
            () = state.shutdown_token.cancelled() => {
                state.input_rx.close();
                state.command_rx.close();
                let shutdown_deadline =
                    tokio::time::Instant::now() + self.config.shutdown_drain_grace;
                state.deadline = Some(shutdown_deadline);
                tokio::select! {
                    biased;
                    () = tokio::time::sleep_until(shutdown_deadline) => {
                        state.drain_token.cancel();
                        InputProcessOutcome::GraceElapsed
                    }
                    result = &mut process => InputProcessOutcome::Completed(result),
                }
            }
            result = &mut process => InputProcessOutcome::Completed(result),
        }
    }
}
