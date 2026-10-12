//! Effect-executor shutdown accounting: join supervision, in-flight
//! counting, and drain wakeups.

use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;

#[derive(Debug)]
pub(crate) enum EffectExecutorFailure {
    Effect(crate::effect_result::EffectFailure),
    TaskJoin(tokio::task::JoinError),
}
impl std::fmt::Display for EffectExecutorFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Effect(source) => write!(formatter, "effect execution failed: {source}"),
            Self::TaskJoin(source) => write!(formatter, "effect task join failed: {source}"),
        }
    }
}

impl std::error::Error for EffectExecutorFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Effect(source) => Some(source),
            Self::TaskJoin(source) => Some(source),
        }
    }
}

pub(super) struct EffectExecutorControl<'a> {
    pub(super) pending_effect_batches: &'a AtomicUsize,
    pub(super) in_flight_effects: &'a AtomicUsize,
    pub(super) executor_quiescence_notify: &'a tokio::sync::Notify,
    pub(super) drain_effects_on_shutdown: bool,
    pub(super) effect_shutdown: &'a tokio_util::sync::CancellationToken,
    pub(super) runtime_shutdown: &'a tokio_util::sync::CancellationToken,
}

impl SillageRuntime {
    pub(super) fn supervise_effect_join(
        result: Result<Result<(), crate::effect_result::EffectFailure>, tokio::task::JoinError>,
        control: &EffectExecutorControl<'_>,
        first_failure: &mut Option<EffectExecutorFailure>,
        cleanup_cancellation: bool,
    ) {
        control.in_flight_effects.fetch_sub(1, Ordering::AcqRel);
        Self::publish_quiescence(
            control.pending_effect_batches,
            control.in_flight_effects,
            control.executor_quiescence_notify,
        );
        let failure = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => EffectExecutorFailure::Effect(error),
            Err(error) if cleanup_cancellation && error.is_cancelled() => return,
            Err(error) => EffectExecutorFailure::TaskJoin(error),
        };
        if first_failure.is_none() {
            *first_failure = Some(failure);
        }
        control.effect_shutdown.cancel();
        control.runtime_shutdown.cancel();
    }

    pub(super) fn spawn_effect_task(
        in_flight: &mut tokio::task::JoinSet<Result<(), crate::effect_result::EffectFailure>>,
        in_flight_effects: &AtomicUsize,
        context: EffectExecutionContext,
        work: EffectWork,
        lane: Arc<tokio::sync::Semaphore>,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) {
        in_flight_effects.fetch_add(1, Ordering::AcqRel);
        in_flight.spawn(async move {
            let permit = tokio::select! {
                biased;
                () = effect_shutdown.cancelled() => return Ok(()),
                permit = lane.acquire_owned() => match permit {
                    Ok(permit) => permit,
                    Err(_) => return Ok(()),
                },
            };
            let result = match work {
                EffectWork::Pending(effect) => context.execute_with_retries(effect).await,
                EffectWork::Prepared(effect) => {
                    context.execute_prepared_with_watchdog(effect).await
                }
                EffectWork::FullTextBatch(requests) => {
                    context
                        .execute_index_full_text_batch_with_retries(requests)
                        .await
                }
            };
            let result = match result {
                Err(error)
                    if SillageRuntime::supervise_effect_failure(
                        &error,
                        &effect_shutdown,
                        &runtime_shutdown,
                    ) =>
                {
                    Err(error)
                }
                _ => Ok(()),
            };
            drop(permit);
            result
        });
    }

    pub(super) async fn finish_effect_executor(
        in_flight: &mut tokio::task::JoinSet<Result<(), crate::effect_result::EffectFailure>>,
        receiver: &mut tokio::sync::mpsc::Receiver<crate::effect_dispatch::EffectBatch>,
        control: &EffectExecutorControl<'_>,
        mut first_failure: Option<EffectExecutorFailure>,
    ) -> Option<EffectExecutorFailure> {
        let mut cleanup_cancellation =
            !control.drain_effects_on_shutdown || control.effect_shutdown.is_cancelled();
        if cleanup_cancellation {
            in_flight.abort_all();
        }
        while let Some(result) = in_flight.join_next().await {
            Self::supervise_effect_join(result, control, &mut first_failure, cleanup_cancellation);
            if !cleanup_cancellation && first_failure.is_some() {
                cleanup_cancellation = true;
                in_flight.abort_all();
            }
        }
        while receiver.try_recv().is_ok() {
            control
                .pending_effect_batches
                .fetch_sub(1, Ordering::AcqRel);
        }
        Self::publish_quiescence(
            control.pending_effect_batches,
            control.in_flight_effects,
            control.executor_quiescence_notify,
        );
        first_failure
    }

    pub(super) fn publish_quiescence(
        pending_effect_batches: &AtomicUsize,
        in_flight_effects: &AtomicUsize,
        executor_quiescence_notify: &tokio::sync::Notify,
    ) {
        if pending_effect_batches.load(Ordering::Acquire) == 0
            && in_flight_effects.load(Ordering::Acquire) == 0
        {
            executor_quiescence_notify.notify_one();
        }
    }
}
