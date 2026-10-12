use crate::effect_dispatch::EffectBatch;
use crate::effect_result::EffectFailure;
use crate::runtime::SillageRuntime;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

impl SillageRuntime {
    fn prepare_effect_executor_wait(
        in_flight: &mut tokio::task::JoinSet<Result<(), EffectFailure>>,
        control: &crate::effect_executor_shutdown::EffectExecutorControl<'_>,
        first_failure: &mut Option<crate::effect_executor_shutdown::EffectExecutorFailure>,
    ) -> Option<bool> {
        while let Some(result) = in_flight.try_join_next() {
            Self::supervise_effect_join(result, control, first_failure, false);
            if first_failure.is_some() {
                return None;
            }
        }
        let has_in_flight = !in_flight.is_empty();
        Self::publish_quiescence(
            control.pending_effect_batches,
            control.in_flight_effects,
            control.executor_quiescence_notify,
        );
        Some(has_in_flight)
    }

    pub(crate) fn spawn_effect_executor(
        &self,
        mut receiver: mpsc::Receiver<EffectBatch>,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<Result<(), crate::effect_executor_shutdown::EffectExecutorFailure>>
    {
        let execution_context = self.effect_execution_context();
        let max_concurrent_effects = self.config.max_concurrent_effects;
        let drain_effects_on_shutdown = self.config.drain_effects_on_shutdown;
        #[cfg(test)]
        let test_pre_failed_effect_task = self.test_pre_failed_effect_task;
        let pending_effect_batches = std::sync::Arc::clone(&self.pending_effect_batches);
        let in_flight_effects = std::sync::Arc::clone(&self.in_flight_effects);
        let executor_quiescence_notify = std::sync::Arc::clone(&self.executor_quiescence_notify);
        tokio::spawn(async move {
            let lanes = super::EffectLanes {
                main: std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent_effects)),
                vector: std::sync::Arc::new(tokio::sync::Semaphore::new(
                    super::VECTOR_LANE_PERMITS,
                )),
            };
            let mut in_flight = tokio::task::JoinSet::<Result<(), EffectFailure>>::new();
            let mut first_failure = None;
            let control = crate::effect_executor_shutdown::EffectExecutorControl {
                pending_effect_batches: &pending_effect_batches,
                in_flight_effects: &in_flight_effects,
                executor_quiescence_notify: &executor_quiescence_notify,
                drain_effects_on_shutdown,
                effect_shutdown: &effect_shutdown,
                runtime_shutdown: &runtime_shutdown,
            };
            Self::publish_quiescence(
                &pending_effect_batches,
                &in_flight_effects,
                &executor_quiescence_notify,
            );
            #[cfg(test)]
            if test_pre_failed_effect_task {
                in_flight_effects.fetch_add(1, Ordering::AcqRel);
                in_flight
                    .spawn(std::future::pending::<Result<(), EffectFailure>>())
                    .abort();
            }
            'executor: loop {
                let Some(has_in_flight) = Self::prepare_effect_executor_wait(
                    &mut in_flight,
                    &control,
                    &mut first_failure,
                ) else {
                    break 'executor;
                };
                let message = tokio::select! {
                    biased;
                    () = effect_shutdown.cancelled() => break,
                    join_result = in_flight.join_next(), if has_in_flight => {
                        if let Some(result) = join_result {
                            Self::supervise_effect_join(
                                result,
                                &control,
                                &mut first_failure,
                                false,
                            );
                            if first_failure.is_some() {
                                break 'executor;
                            }
                        }
                        continue;
                    },
                    message = receiver.recv() => message,
                };
                let Some((effects, merged_batches)) =
                    Self::merge_queued_effect_batches(message, &mut receiver)
                else {
                    break;
                };
                if effect_shutdown.is_cancelled() {
                    pending_effect_batches.fetch_sub(merged_batches, Ordering::AcqRel);
                    break;
                }
                let admission_error = Self::admit_effect_batch(
                    &mut in_flight,
                    &in_flight_effects,
                    &lanes,
                    &execution_context,
                    effects,
                    &effect_shutdown,
                    &runtime_shutdown,
                )
                .await;
                pending_effect_batches.fetch_sub(merged_batches, Ordering::AcqRel);
                if let Some(error) = admission_error {
                    first_failure =
                        Some(crate::effect_executor_shutdown::EffectExecutorFailure::Effect(error));
                    break 'executor;
                }
                Self::publish_quiescence(
                    &pending_effect_batches,
                    &in_flight_effects,
                    &executor_quiescence_notify,
                );
            }
            match Self::finish_effect_executor(
                &mut in_flight,
                &mut receiver,
                &control,
                first_failure,
            )
            .await
            {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }
}
