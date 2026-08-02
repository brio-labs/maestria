use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use crate::effect_result::EffectFailure;
use crate::runtime::MaestriaRuntime;
use maestria_domain::{KernelState, MaestriaEffect, ValidationReportId};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::mpsc;

/// Outcome of admitting one effect from a received batch.
enum AdmitOutcome {
    /// Spawned the effect; continue with the next item in the batch.
    Continue,
    /// The effect was a persist event executed inline; continue.
    Persisted,
    /// Stop the whole executor immediately (persist failure).
    Stop,
    /// Shutdown/semaphore closed before admission; drop the rest.
    DropBatch,
}

impl MaestriaRuntime {
    pub(crate) fn seed_next_validation_report_id(state: &KernelState) -> u64 {
        state
            .validation_reports
            .keys()
            .map(|id| id.value())
            .max()
            .map_or(1, |value| value.saturating_add(1))
    }

    /// Seed the command correlation-id counter from persisted state so the
    /// per-process counter never reuses a correlation id already recorded in
    /// the event log (R27: persisted identity namespaces must not be coupled
    /// to an ephemeral allocation scheme).
    pub(crate) fn seed_next_command_id(state: &KernelState) -> u64 {
        let request_max = state
            .model_agent_requests
            .values()
            .map(|request| request.correlation_id.value())
            .max();
        let result_max = state
            .model_agent_results
            .values()
            .map(|result| result.correlation_id().value())
            .max();
        let highest = match (request_max, result_max) {
            (Some(left), Some(right)) => left.max(right),
            (Some(value), None) | (None, Some(value)) => value,
            (None, None) => 0,
        };
        highest.saturating_add(1).max(1)
    }

    fn supervise_effect_failure(
        error: EffectFailure,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) -> bool {
        match error {
            EffectFailure::Denied(reason) => {
                tracing::warn!(%reason, "spawned effect denied; continuing runtime execution");
                false
            }
            EffectFailure::RequiresApproval(reason) => {
                tracing::info!(
                    %reason,
                    "spawned effect is awaiting approval; continuing runtime execution"
                );
                false
            }
            EffectFailure::ApprovalLookup(error) => {
                tracing::error!(
                    %error,
                    "spawned effect approval lookup failed; cancelling runtime execution"
                );
                effect_shutdown.cancel();
                runtime_shutdown.cancel();
                true
            }
            EffectFailure::Failed(reason) => {
                tracing::error!(
                    reason = %reason,
                    "spawned effect failed; cancelling runtime execution"
                );
                effect_shutdown.cancel();
                runtime_shutdown.cancel();
                true
            }
            EffectFailure::Degraded(reason) => {
                tracing::warn!(
                    %reason,
                    "spawned effect degraded but remains recoverable; continuing runtime execution"
                );
                false
            }
        }
    }

    /// Assign the next validation-report id to a pending `RunValidation`
    /// work item before admission.
    fn assign_validation_report_id(
        work: &mut EffectWork,
        next_validation_report_id: &std::sync::atomic::AtomicU64,
    ) {
        if let EffectWork::Pending(MaestriaEffect::RunValidation(request)) = work {
            request.validation_report_id =
                ValidationReportId::new(next_validation_report_id.fetch_add(1, Ordering::Relaxed));
        }
    }

    /// Run a pending `PersistEvent` inline within the executor, bypassing
    /// semaphore admission. Returns `Ok(None)` when the event was persisted,
    /// `Ok(Some(work))` when the item is not a pending persist event and
    /// should be admitted normally, and `Err` when persistence failed (the
    /// caller must stop the executor).
    async fn run_persist_event(
        context: EffectExecutionContext,
        work: EffectWork,
    ) -> Result<Option<EffectWork>, EffectFailure> {
        match work {
            EffectWork::Pending(effect @ MaestriaEffect::PersistEvent { .. }) => {
                context.execute_with_retries(effect).await.map(|()| None)
            }
            other => Ok(Some(other)),
        }
    }

    fn supervise_effect_join(
        result: Result<(), tokio::task::JoinError>,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) {
        let Err(error) = result else {
            return;
        };
        tracing::error!(
            %error,
            task_panicked = error.is_panic(),
            task_cancelled = error.is_cancelled(),
            "spawned effect task join failed; cancelling runtime execution"
        );
        effect_shutdown.cancel();
        runtime_shutdown.cancel();
    }

    fn spawn_effect_task(
        in_flight: &mut tokio::task::JoinSet<()>,
        context: EffectExecutionContext,
        work: EffectWork,
        permit: tokio::sync::OwnedSemaphorePermit,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) {
        in_flight.spawn(async move {
            let result = match work {
                EffectWork::Pending(effect) => context.execute_with_retries(effect).await,
                EffectWork::Prepared(effect) => {
                    context.execute_prepared_with_watchdog(effect).await
                }
            };
            if let Err(error) = result {
                Self::supervise_effect_failure(error, &effect_shutdown, &runtime_shutdown);
            }
            drop(permit);
        });
    }

    async fn finish_effect_executor(
        in_flight: &mut tokio::task::JoinSet<()>,
        drain_effects_on_shutdown: bool,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) {
        if drain_effects_on_shutdown && !effect_shutdown.is_cancelled() {
            while let Some(result) = in_flight.join_next().await {
                Self::supervise_effect_join(result, effect_shutdown, runtime_shutdown);
            }
        } else {
            in_flight.abort_all();
            while let Some(result) = in_flight.join_next().await {
                Self::supervise_effect_join(result, effect_shutdown, runtime_shutdown);
            }
        }
    }
    pub(crate) fn effect_execution_context(&self) -> EffectExecutionContext {
        EffectExecutionContext {
            adapters: Arc::clone(&self.adapters),
            governance: Arc::clone(&self.governance),
            profile: self.config.profile,
            scope: self.config.scope.clone(),
            scope_id: self.config.scope_id,
            state: Arc::clone(&self.state),
            input_tx: self.input_tx.clone(),
            embedding_model: self.config.embedding_model.clone(),
            feedback_acks: Arc::clone(&self.feedback_acks),
            journal_recovery_claims: Arc::clone(&self.journal_recovery_claims),
            default_effect_timeout: self.config.default_effect_timeout,
            max_retries: self.config.max_retries,
        }
    }

    /// Admit one effect: assign its validation-report id, execute inline
    /// persist events, acquire a semaphore permit, and spawn the task.
    async fn admit_effect(
        in_flight: &mut tokio::task::JoinSet<()>,
        semaphore: &Arc<tokio::sync::Semaphore>,
        execution_context: EffectExecutionContext,
        next_validation_report_id: &std::sync::atomic::AtomicU64,
        mut work: EffectWork,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) -> AdmitOutcome {
        if effect_shutdown.is_cancelled() {
            return AdmitOutcome::DropBatch;
        }
        Self::assign_validation_report_id(&mut work, next_validation_report_id);
        let work = match Self::run_persist_event(execution_context.clone(), work).await {
            Ok(None) => return AdmitOutcome::Persisted,
            Ok(Some(work)) => work,
            Err(error) => {
                tracing::error!(%error, "persist event failed; stopping effect executor");
                effect_shutdown.cancel();
                runtime_shutdown.cancel();
                return AdmitOutcome::Stop;
            }
        };
        let permit = tokio::select! {
            biased;
            () = effect_shutdown.cancelled() => return AdmitOutcome::DropBatch,
            permit = Arc::clone(semaphore).acquire_owned() => {
                match permit {
                    Ok(permit) => permit,
                    Err(_) => return AdmitOutcome::DropBatch,
                }
            }
        };
        Self::spawn_effect_task(
            in_flight,
            execution_context,
            work,
            permit,
            effect_shutdown.clone(),
            runtime_shutdown.clone(),
        );
        AdmitOutcome::Continue
    }

    pub(crate) fn spawn_effect_executor(
        &self,
        mut receiver: mpsc::Receiver<crate::effect_dispatch::EffectBatch>,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let execution_context = self.effect_execution_context();
        let max_concurrent_effects = self.config.max_concurrent_effects;
        let next_validation_report_id = Arc::clone(&self.next_validation_report_id);
        let drain_effects_on_shutdown = self.config.drain_effects_on_shutdown;
        #[cfg(test)]
        let test_pre_failed_effect_task = self.test_pre_failed_effect_task;
        tokio::spawn(async move {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_effects));
            let mut in_flight = tokio::task::JoinSet::new();
            #[cfg(test)]
            if test_pre_failed_effect_task {
                let abort_handle = in_flight.spawn(std::future::pending::<()>());
                abort_handle.abort();
            }
            'executor: loop {
                while let Some(result) = in_flight.try_join_next() {
                    Self::supervise_effect_join(result, &effect_shutdown, &runtime_shutdown);
                }
                let has_in_flight = !in_flight.is_empty();
                let message = tokio::select! {
                    biased;
                    () = effect_shutdown.cancelled() => break,
                    join_result = in_flight.join_next(), if has_in_flight => {
                        if let Some(result) = join_result {
                            Self::supervise_effect_join(
                                result,
                                &effect_shutdown,
                                &runtime_shutdown,
                            );
                        }
                        continue;
                    },
                    message = receiver.recv() => message,
                };
                let Some(effects) = message else { break };
                if effect_shutdown.is_cancelled() {
                    break;
                }
                let mut remaining = effects.len();
                for work in effects {
                    remaining = remaining.saturating_sub(1);
                    match Self::admit_effect(
                        &mut in_flight,
                        &semaphore,
                        execution_context.clone(),
                        &next_validation_report_id,
                        work,
                        &effect_shutdown,
                        &runtime_shutdown,
                    )
                    .await
                    {
                        AdmitOutcome::Continue | AdmitOutcome::Persisted => {}
                        AdmitOutcome::Stop => break 'executor,
                        AdmitOutcome::DropBatch => {
                            tracing::warn!(
                                dropped_effects = remaining,
                                "effect executor dropped admitted effects"
                            );
                            break;
                        }
                    }
                }
            }
            Self::finish_effect_executor(
                &mut in_flight,
                drain_effects_on_shutdown,
                &effect_shutdown,
                &runtime_shutdown,
            )
            .await;
        })
    }

    pub(crate) async fn wait_for_validation_report(
        &self,
        report_id: maestria_domain::ValidationReportId,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        crate::persistence_barrier::wait_for_event(
            &*self.adapters.event_log,
            self.config.default_effect_timeout,
            shutdown_token,
            "validation report barrier",
            crate::persistence_barrier::validation_report_created(report_id),
        )
        .await
    }

    pub(crate) async fn wait_for_event_persistence(
        &self,
        event_id: maestria_domain::EventId,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        crate::persistence_barrier::wait_for_event(
            &*self.adapters.event_log,
            self.config.default_effect_timeout,
            shutdown_token,
            "event persistence barrier",
            crate::persistence_barrier::event_persisted(event_id),
        )
        .await
    }

    pub(crate) async fn wait_for_approval_resolution(
        &self,
        event_id: maestria_domain::EventId,
        approval_id: maestria_domain::ApprovalId,
        approved: bool,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        crate::persistence_barrier::wait_for_event(
            &*self.adapters.event_log,
            self.config.default_effect_timeout,
            shutdown_token,
            "approval persistence barrier",
            crate::persistence_barrier::approval_resolved(
                event_id,
                approval_id,
                approved,
                &*self.adapters.approval_repo,
            ),
        )
        .await
    }
}
