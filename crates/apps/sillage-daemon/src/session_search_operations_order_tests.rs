use std::{
    future::Future,
    sync::{
        Arc, TryLockError,
        atomic::{AtomicU8, Ordering},
    },
    task::Poll,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::OperationTracker;

const NOT_DROPPED: u8 = 0;
const DROPPED_WITH_UNLOCKED_TRACKER: u8 = 1;
const DROPPED_WITH_LOCKED_TRACKER: u8 = 2;
const DROPPED_WITH_POISONED_TRACKER: u8 = 3;

struct DropProbe {
    tracker: Arc<OperationTracker>,
    observation: Arc<AtomicU8>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        let observation = match self.tracker.state.try_lock() {
            Ok(state) => {
                drop(state);
                DROPPED_WITH_UNLOCKED_TRACKER
            }
            Err(TryLockError::WouldBlock) => DROPPED_WITH_LOCKED_TRACKER,
            Err(TryLockError::Poisoned(state)) => {
                drop(state.into_inner());
                DROPPED_WITH_POISONED_TRACKER
            }
        };
        self.observation.store(observation, Ordering::Release);
    }
}

#[tokio::test]
async fn closed_epoch_destroys_successful_result_after_tracker_unlock() -> Result<()> {
    let tracker = Arc::new(OperationTracker::new());
    let observation = Arc::new(AtomicU8::new(NOT_DROPPED));
    let (reached_sender, reached_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let operation_tracker = tracker.clone();
    let operation_observation = observation.clone();
    let operation = async move {
        let _ = reached_sender.send(());
        release_receiver.await.map_err(anyhow::Error::new)?;
        Ok(DropProbe {
            tracker: operation_tracker,
            observation: operation_observation,
        })
    };
    let caller_tracker = tracker.clone();
    let caller = tokio::spawn(async move {
        caller_tracker
            .track(CancellationToken::new(), operation)
            .await
    });
    let reached = tokio::time::timeout(Duration::from_secs(10), reached_receiver)
        .await
        .is_ok_and(|signal| signal.is_ok());
    tracker.close_and_cancel();
    let _ = release_sender.send(());
    let caller_result = caller
        .await
        .map_err(anyhow::Error::new)
        .context("closed-epoch caller failed to join");
    tracker.wait().await;
    let caller_result = caller_result?;
    let typed_cancellation = matches!(
        &caller_result,
        Err(error)
            if error.downcast_ref::<crate::SessionOperationSettlement>()
                == Some(&crate::SessionOperationSettlement::RoutineCancellation)
    );
    let observed = observation.load(Ordering::Acquire);
    if !reached || !typed_cancellation || observed != DROPPED_WITH_UNLOCKED_TRACKER {
        bail!(
            "closed-epoch result drop ordering failed (accepted gate reached: {reached}; \
             typed cancellation delivered: {typed_cancellation}; destructor observation: \
             {observed})"
        );
    }
    Ok(())
}

#[tokio::test]
async fn failed_result_delivery_destroys_value_after_tracker_unlock() -> Result<()> {
    let tracker = Arc::new(OperationTracker::new());
    let observation = Arc::new(AtomicU8::new(NOT_DROPPED));
    let (reached_sender, reached_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let operation_tracker = tracker.clone();
    let operation_observation = observation.clone();
    let operation = async move {
        let _ = reached_sender.send(());
        release_receiver.await.map_err(anyhow::Error::new)?;
        Ok(DropProbe {
            tracker: operation_tracker,
            observation: operation_observation,
        })
    };
    let mut caller = Box::pin(tracker.track(CancellationToken::new(), operation));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(caller.as_mut().poll(context))).await;
    let reached = tokio::time::timeout(Duration::from_secs(10), reached_receiver)
        .await
        .is_ok_and(|signal| signal.is_ok());
    drop(caller);
    let _ = release_sender.send(());
    tracker.wait().await;
    let observed = observation.load(Ordering::Acquire);
    if !matches!(first_poll, Poll::Pending) || !reached || observed != DROPPED_WITH_UNLOCKED_TRACKER
    {
        bail!(
            "undeliverable result drop ordering failed (caller accepted: {}; gate reached: \
             {reached}; destructor observation: {observed})",
            matches!(first_poll, Poll::Pending)
        );
    }
    Ok(())
}
