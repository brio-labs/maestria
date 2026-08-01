use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use crate::effect_result::EffectFailure;
use crate::runtime::MaestriaRuntime;
use maestria_domain::{KernelState, MaestriaEffect, ValidationReportId};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::mpsc;

impl MaestriaRuntime {
    pub(crate) fn seed_next_validation_report_id(state: &KernelState) -> u64 {
        state
            .validation_reports
            .keys()
            .map(|id| id.value())
            .max()
            .map_or(1, |value| value.saturating_add(1))
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
                for mut work in effects {
                    if effect_shutdown.is_cancelled() {
                        break;
                    }
                    if let EffectWork::Pending(MaestriaEffect::RunValidation(request)) = &mut work {
                        request.validation_report_id = ValidationReportId::new(
                            next_validation_report_id.fetch_add(1, Ordering::Relaxed),
                        );
                    }
                    let context = execution_context.clone();
                    if let EffectWork::Pending(MaestriaEffect::PersistEvent { .. }) = &work {
                        let EffectWork::Pending(effect) = work else {
                            continue;
                        };
                        if let Err(error) = context.execute_with_retries(effect).await {
                            tracing::error!(
                                %error,
                                "persist event failed; stopping effect executor"
                            );
                            effect_shutdown.cancel();
                            runtime_shutdown.cancel();
                            break 'executor;
                        }
                        continue;
                    }
                    let permit = tokio::select! {
                        biased;
                        () = effect_shutdown.cancelled() => break,
                        permit = Arc::clone(&semaphore).acquire_owned() => {
                            match permit {
                                Ok(permit) => permit,
                                Err(_) => break,
                            }
                        }
                    };
                    Self::spawn_effect_task(
                        &mut in_flight,
                        context,
                        work,
                        permit,
                        effect_shutdown.clone(),
                        runtime_shutdown.clone(),
                    );
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
        let check = async {
            loop {
                if shutdown_token.is_cancelled() {
                    return false;
                }
                match self
                    .adapters
                    .event_log
                    .scan(maestria_ports::EventFilter { artifact_id: None })
                {
                    Ok(events) => {
                        if events.iter().any(|env| {
                            matches!(
                                &env.event,
                                maestria_domain::DomainEvent::ValidationReportCreated {
                                    report_id: id,
                                    ..
                                } if *id == report_id
                            )
                        }) {
                            return true;
                        }
                    }
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "failed to scan event log during validation report barrier"
                        );
                        return false;
                    }
                }
                tokio::select! {
                    () = shutdown_token.cancelled() => return false,
                    () = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
                }
            }
        };
        matches!(
            tokio::time::timeout(self.config.default_effect_timeout, check).await,
            Ok(true)
        )
    }

    pub(crate) async fn wait_for_event_persistence(
        &self,
        event_id: maestria_domain::EventId,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let check = async {
            loop {
                if shutdown_token.is_cancelled() {
                    return false;
                }
                match self
                    .adapters
                    .event_log
                    .scan(maestria_ports::EventFilter { artifact_id: None })
                {
                    Ok(events) if events.iter().any(|event| event.id == event_id) => return true,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "failed to scan event log during persistence barrier");
                        return false;
                    }
                }
                tokio::select! {
                    () = shutdown_token.cancelled() => return false,
                    () = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
                }
            }
        };
        matches!(
            tokio::time::timeout(self.config.default_effect_timeout, check).await,
            Ok(true)
        )
    }
    pub(crate) async fn wait_for_approval_resolution(
        &self,
        event_id: maestria_domain::EventId,
        approval_id: maestria_domain::ApprovalId,
        approved: bool,
        shutdown_token: &tokio_util::sync::CancellationToken,
    ) -> bool {
        let check = async {
            loop {
                if shutdown_token.is_cancelled() {
                    return false;
                }
                let event_persisted = match self
                    .adapters
                    .event_log
                    .scan(maestria_ports::EventFilter { artifact_id: None })
                {
                    Ok(events) => events.iter().any(|event| {
                        event.id == event_id
                            && matches!(
                                &event.event,
                                maestria_domain::DomainEvent::ApprovalRecorded {
                                    approval_id: id,
                                    outcome,
                                } if *id == approval_id && outcome.approved() == approved
                            )
                    }),
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "failed to scan event log during approval persistence barrier"
                        );
                        return false;
                    }
                };
                if event_persisted {
                    let projected = match self.adapters.approval_repo.find_by_id(approval_id) {
                        Ok(Some(record)) => {
                            record.status
                                == if approved {
                                    maestria_ports::ApprovalStatus::Approved
                                } else {
                                    maestria_ports::ApprovalStatus::Denied
                                }
                        }
                        Ok(None) => false,
                        Err(error) => {
                            tracing::error!(
                                %error,
                                "failed to read approval projection during persistence barrier"
                            );
                            return false;
                        }
                    };
                    if projected {
                        return true;
                    }
                }
                tokio::select! {
                    () = shutdown_token.cancelled() => return false,
                    () = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
                }
            }
        };
        matches!(
            tokio::time::timeout(self.config.default_effect_timeout, check).await,
            Ok(true)
        )
    }
}
