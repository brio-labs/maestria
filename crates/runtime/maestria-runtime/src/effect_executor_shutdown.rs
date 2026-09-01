//! Effect-executor shutdown accounting: join supervision, in-flight
//! counting, and quiescence publication for the shutdown drain.

use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::*;

impl MaestriaRuntime {
    pub(super) fn supervise_effect_join(
        result: Result<(), tokio::task::JoinError>,
        pending_effect_batches: &AtomicUsize,
        in_flight_effects: &AtomicUsize,
        executor_quiescent: &AtomicBool,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) {
        // Every joined task (success, panic, or cancellation) releases its
        // in-flight slot; the quiescence publication then reflects the new
        // occupancy.
        in_flight_effects.fetch_sub(1, Ordering::Relaxed);
        Self::publish_quiescence(
            pending_effect_batches,
            in_flight_effects,
            executor_quiescent,
        );
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

    pub(super) fn spawn_effect_task(
        in_flight: &mut tokio::task::JoinSet<()>,
        in_flight_effects: &AtomicUsize,
        context: EffectExecutionContext,
        work: EffectWork,
        lane: Arc<tokio::sync::Semaphore>,
        effect_shutdown: tokio_util::sync::CancellationToken,
        runtime_shutdown: tokio_util::sync::CancellationToken,
    ) {
        in_flight_effects.fetch_add(1, Ordering::Relaxed);
        in_flight.spawn(async move {
            // Permit acquired inside the task so batch consumption never
            // blocks on the semaphore; the lane still bounds concurrent
            // executions and the effect channel cannot back up.
            let permit = tokio::select! {
                biased;
                () = effect_shutdown.cancelled() => return,
                permit = lane.acquire_owned() => match permit {
                    Ok(permit) => permit,
                    Err(_) => return,
                },
            };
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

    pub(super) async fn finish_effect_executor(
        in_flight: &mut tokio::task::JoinSet<()>,
        pending_effect_batches: &AtomicUsize,
        in_flight_effects: &AtomicUsize,
        executor_quiescent: &AtomicBool,
        drain_effects_on_shutdown: bool,
        effect_shutdown: &tokio_util::sync::CancellationToken,
        runtime_shutdown: &tokio_util::sync::CancellationToken,
    ) {
        if drain_effects_on_shutdown && !effect_shutdown.is_cancelled() {
            while let Some(result) = in_flight.join_next().await {
                Self::supervise_effect_join(
                    result,
                    pending_effect_batches,
                    in_flight_effects,
                    executor_quiescent,
                    effect_shutdown,
                    runtime_shutdown,
                );
            }
        } else {
            in_flight.abort_all();
            while let Some(result) = in_flight.join_next().await {
                Self::supervise_effect_join(
                    result,
                    pending_effect_batches,
                    in_flight_effects,
                    executor_quiescent,
                    effect_shutdown,
                    runtime_shutdown,
                );
            }
        }
        // The executor is done: no queued batch will ever be taken and no
        // effect is running. The shutdown drain must observe this
        // immediately instead of waiting out its grace.
        executor_quiescent.store(true, Ordering::Relaxed);
    }

    /// Publish the executor's quiescence: no batch queued, no effect
    /// running. Called after every state change so the shutdown drain's
    /// exit condition tracks the executor exactly.
    pub(super) fn publish_quiescence(
        pending_effect_batches: &AtomicUsize,
        in_flight_effects: &AtomicUsize,
        executor_quiescent: &AtomicBool,
    ) {
        let quiescent = pending_effect_batches.load(Ordering::Relaxed) == 0
            && in_flight_effects.load(Ordering::Relaxed) == 0;
        executor_quiescent.store(quiescent, Ordering::Relaxed);
    }
}
