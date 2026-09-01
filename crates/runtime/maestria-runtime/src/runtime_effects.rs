use crate::config::EffectExecutionContext;
use crate::effect_dispatch::{EffectBatch, EffectWork};
use crate::effect_execution_dispatch::PreparedEffect;
use crate::effect_result::EffectFailure;
use crate::runtime::MaestriaRuntime;
use maestria_domain::{KernelState, MaestriaEffect};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::mpsc;

/// Dedicated lane for `IndexVector` effects: a degraded vector flood must
/// never occupy the main effect semaphore to the exclusion of full-text and
/// parse effects. Eight permits keep provider-backed embedding runs
/// concurrent: measured dense-ingest throughput saturates near the host
/// core count, and higher permit counts over-subscribe ONNX inference.
const VECTOR_LANE_PERMITS: usize = 8;

/// Effect semaphores: the main lane carries every effect except vector
/// indexing, which runs under [`VECTOR_LANE_PERMITS`].
struct EffectLanes {
    main: Arc<tokio::sync::Semaphore>,
    vector: Arc<tokio::sync::Semaphore>,
}

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

    pub(super) fn supervise_effect_failure(
        error: EffectFailure,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let is_fatal = error.is_fatal();
        Self::log_supervised_effect_failure(&error);
        if is_fatal {
            effect_shutdown.cancel();
            runtime_shutdown.cancel();
            true
        } else {
            false
        }
    }

    fn log_supervised_effect_failure(error: &EffectFailure) {
        match error {
            EffectFailure::ApprovalLookup(_) | EffectFailure::Failed(_) => {
                tracing::error!(%error, "spawned effect failed; cancelling runtime execution");
            }
            EffectFailure::RequiresApproval(_) => {
                tracing::info!(
                    %error,
                    "spawned effect is awaiting approval; continuing runtime execution"
                );
            }
            EffectFailure::Denied(_) | EffectFailure::Degraded(_) => {
                tracing::warn!(
                    %error,
                    "spawned effect did not complete; continuing runtime execution"
                );
            }
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
            degraded_vector_artifacts: Arc::clone(&self.degraded_vector_artifacts),
            full_text_locks: Arc::clone(&self.full_text_locks),
            default_effect_timeout: self.config.default_effect_timeout,
            max_retries: self.config.max_retries,
        }
    }

    /// Admit one effect: execute inline persist events and spawn the task.
    /// `IndexVector` effects (pending and prepared forms) run under the
    /// dedicated vector lane.
    async fn admit_effect(
        in_flight: &mut tokio::task::JoinSet<()>,
        in_flight_effects: &AtomicUsize,
        lanes: &EffectLanes,
        execution_context: EffectExecutionContext,
        work: EffectWork,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) -> AdmitOutcome {
        if effect_shutdown.is_cancelled() {
            return AdmitOutcome::DropBatch;
        }
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
        let is_vector_effect = matches!(
            &work,
            EffectWork::Pending(MaestriaEffect::IndexArtifactVectors(_))
        ) || matches!(
            &work,
            EffectWork::Prepared(PreparedEffect::Dispatch { effect, .. })
                if matches!(**effect, MaestriaEffect::IndexArtifactVectors(_))
        );
        let lane = if is_vector_effect {
            Arc::clone(&lanes.vector)
        } else {
            Arc::clone(&lanes.main)
        };
        Self::spawn_effect_task(
            in_flight,
            in_flight_effects,
            execution_context,
            work,
            lane,
            effect_shutdown.clone(),
            runtime_shutdown.clone(),
        );
        AdmitOutcome::Continue
    }

    /// Admit a received batch effect by effect. Returns `true` when the
    /// executor must stop (persist failure); drops the batch remainder when
    /// the executor is shutting down.
    async fn admit_effect_batch(
        in_flight: &mut tokio::task::JoinSet<()>,
        in_flight_effects: &AtomicUsize,
        lanes: &EffectLanes,
        execution_context: &EffectExecutionContext,
        effects: EffectBatch,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let mut remaining = effects.len();
        for work in effects {
            remaining = remaining.saturating_sub(1);
            match Self::admit_effect(
                in_flight,
                in_flight_effects,
                lanes,
                execution_context.clone(),
                work,
                effect_shutdown,
                runtime_shutdown,
            )
            .await
            {
                AdmitOutcome::Continue | AdmitOutcome::Persisted => {}
                AdmitOutcome::Stop => return true,
                AdmitOutcome::DropBatch => {
                    tracing::warn!(
                        dropped_effects = remaining,
                        "effect executor dropped admitted effects"
                    );
                    return false;
                }
            }
        }
        false
    }

    pub(crate) fn spawn_effect_executor(
        &self,
        mut receiver: mpsc::Receiver<crate::effect_dispatch::EffectBatch>,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let execution_context = self.effect_execution_context();
        let max_concurrent_effects = self.config.max_concurrent_effects;
        let drain_effects_on_shutdown = self.config.drain_effects_on_shutdown;
        #[cfg(test)]
        let test_pre_failed_effect_task = self.test_pre_failed_effect_task;
        let pending_effect_batches = Arc::clone(&self.pending_effect_batches);
        let in_flight_effects = Arc::clone(&self.in_flight_effects);
        let executor_quiescent = Arc::clone(&self.executor_quiescent);
        tokio::spawn(async move {
            let lanes = EffectLanes {
                main: Arc::new(tokio::sync::Semaphore::new(max_concurrent_effects)),
                vector: Arc::new(tokio::sync::Semaphore::new(VECTOR_LANE_PERMITS)),
            };
            let mut in_flight = tokio::task::JoinSet::new();
            // Publish the initial quiescent state before parking on recv():
            // a quiet runtime's shutdown drain must observe it immediately.
            Self::publish_quiescence(
                &pending_effect_batches,
                &in_flight_effects,
                &executor_quiescent,
            );
            #[cfg(test)]
            if test_pre_failed_effect_task {
                let abort_handle = in_flight.spawn(std::future::pending::<()>());
                abort_handle.abort();
            }
            'executor: loop {
                while let Some(result) = in_flight.try_join_next() {
                    Self::supervise_effect_join(
                        result,
                        &pending_effect_batches,
                        &in_flight_effects,
                        &executor_quiescent,
                        &effect_shutdown,
                        &runtime_shutdown,
                    );
                }
                let has_in_flight = !in_flight.is_empty();
                Self::publish_quiescence(
                    &pending_effect_batches,
                    &in_flight_effects,
                    &executor_quiescent,
                );
                let message = tokio::select! {
                    biased;
                    () = effect_shutdown.cancelled() => break,
                    join_result = in_flight.join_next(), if has_in_flight => {
                        if let Some(result) = join_result {
                            Self::supervise_effect_join(
                                result,
                                &pending_effect_batches,
                                &in_flight_effects,
                                &executor_quiescent,
                                &effect_shutdown,
                                &runtime_shutdown,
                            );
                        }
                        continue;
                    },
                    message = receiver.recv() => message,
                };
                let Some(effects) = message else { break };
                // The batch left the queue and is now this loop's sole
                // responsibility (executed, inline-persisted, or dropped).
                pending_effect_batches.fetch_sub(1, Ordering::Relaxed);
                if effect_shutdown.is_cancelled() {
                    break;
                }
                if Self::admit_effect_batch(
                    &mut in_flight,
                    &in_flight_effects,
                    &lanes,
                    &execution_context,
                    effects,
                    &effect_shutdown,
                    &runtime_shutdown,
                )
                .await
                {
                    break 'executor;
                }
                Self::publish_quiescence(
                    &pending_effect_batches,
                    &in_flight_effects,
                    &executor_quiescent,
                );
            }
            Self::finish_effect_executor(
                &mut in_flight,
                &pending_effect_batches,
                &in_flight_effects,
                &executor_quiescent,
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
            maestria_ports::EventFilter { artifact_id: None },
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
            maestria_ports::EventFilter { artifact_id: None },
            self.config.default_effect_timeout,
            shutdown_token,
            "event persistence barrier",
            crate::persistence_barrier::event_persisted(event_id),
        )
        .await
    }

    pub(crate) async fn wait_for_realm_read_grant_persistence(
        &self,
        event_id: maestria_domain::EventId,
        expected: maestria_domain::RealmReadGrant,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        crate::persistence_barrier::wait_for_event(
            &*self.adapters.event_log,
            maestria_ports::EventFilter { artifact_id: None },
            self.config.default_effect_timeout,
            shutdown_token,
            "realm read grant projection barrier",
            crate::persistence_barrier::realm_read_grant_persisted(
                event_id,
                expected,
                &*self.adapters.realm_read_grant_repo,
            ),
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
            maestria_ports::EventFilter { artifact_id: None },
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
