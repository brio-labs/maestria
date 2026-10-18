use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::{Result, anyhow, bail};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::SessionLifecycleFailure;

#[derive(Default)]
pub(super) struct OperationTracker {
    state: Mutex<TrackerState>,
    idle: Notify,
}

#[derive(Default)]
struct TrackerState {
    accepting: bool,
    next_id: u64,
    active: BTreeMap<u64, CancellationToken>,
    dropped_errors: Vec<anyhow::Error>,
    epoch: u64,
    poison_reported: bool,
    owner_registry_poison_reported: bool,
}

impl OperationTracker {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(TrackerState {
                accepting: true,
                ..TrackerState::default()
            }),
            idle: Notify::new(),
        }
    }
    fn state_lock(&self) -> Result<MutexGuard<'_, TrackerState>> {
        self.state
            .lock()
            .map_err(|_| anyhow::Error::new(SessionLifecycleFailure::OperationState))
    }

    fn recover_state(&self) -> (MutexGuard<'_, TrackerState>, bool) {
        match self.state.lock() {
            Ok(state) => (state, false),
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                record_state_poison(&mut state);
                (state, true)
            }
        }
    }

    pub(super) fn record_owner_registry_poison(&self) {
        let (mut state, _) = self.recover_state();
        if !state.owner_registry_poison_reported {
            state.owner_registry_poison_reported = true;
            state
                .dropped_errors
                .push(SessionLifecycleFailure::OwnerControlRegistry.into());
        }
    }

    pub(super) async fn track<T, F>(
        self: &Arc<Self>,
        cancellation: CancellationToken,
        operation: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = Result<T>> + Send + 'static,
    {
        let mut cancellation_guard = CancelOnDrop::new(cancellation.clone());
        let guard = self.begin(cancellation)?;
        let tracker = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let epoch = guard.epoch;
        tokio::spawn(async move {
            let _guard = guard;
            let operation_result = operation.await;
            let (closed_epoch, state_poisoned) = {
                let (state, state_poisoned) = tracker.recover_state();
                (!state.accepting || state.epoch != epoch, state_poisoned)
            };
            let result = if closed_epoch {
                match operation_result {
                    Ok(value) => {
                        drop(value);
                        Err(crate::SessionOperationSettlement::RoutineCancellation.into())
                    }
                    Err(error) => Err(error),
                }
            } else {
                operation_result
            };
            let result = if state_poisoned {
                match result {
                    Ok(value) => {
                        drop(value);
                        Err(SessionLifecycleFailure::OperationState.into())
                    }
                    Err(error) => Err(error.context(SessionLifecycleFailure::OperationState)),
                }
            } else {
                result
            };
            if let Err(result) = sender.send(result) {
                match result {
                    Ok(value) => drop(value),
                    Err(error) => tracker.recover_state().0.dropped_errors.push(error),
                }
            }
        });
        let result = receiver
            .await
            .map_err(|_| anyhow!("tracked session supervisor ended without a result"))?;
        cancellation_guard.complete();
        result
    }

    fn begin(self: &Arc<Self>, cancellation: CancellationToken) -> Result<OperationGuard> {
        let mut state = self.state_lock()?;
        if !state.accepting {
            bail!("session operation admission is retired");
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("session operation identity overflow"))?;
        state.next_id = id;
        state.active.insert(id, cancellation);
        Ok(OperationGuard {
            tracker: self.clone(),
            id,
            epoch: state.epoch,
        })
    }

    pub(super) fn cancel_active(&self) {
        let (mut state, _) = self.recover_state();
        advance_epoch(&mut state);
        for cancellation in state.active.values() {
            cancellation.cancel();
        }
    }

    pub(super) fn close_and_cancel(&self) {
        let (mut state, _) = self.recover_state();
        state.accepting = false;
        advance_epoch(&mut state);
        for cancellation in state.active.values() {
            cancellation.cancel();
        }
    }

    pub(super) fn with_open_admission<T>(
        &self,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let state = self.state_lock()?;
        if !state.accepting {
            bail!("session operation admission is retired");
        }
        operation()
    }

    pub(super) fn is_idle(&self) -> Result<bool> {
        let state = self.state_lock()?;
        Ok(state.active.is_empty())
    }

    pub(super) async fn wait(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let idle = self.recover_state().0.active.is_empty();
            if idle {
                return;
            }
            notified.await;
        }
    }

    pub(super) async fn wait_and_collect_errors(&self) -> Vec<anyhow::Error> {
        self.wait().await;
        let (mut state, _) = self.recover_state();
        std::mem::take(&mut state.dropped_errors)
    }
}

struct OperationGuard {
    tracker: Arc<OperationTracker>,
    id: u64,
    epoch: u64,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let (mut state, _) = self.tracker.recover_state();
        state.active.remove(&self.id);
        if state.active.is_empty() {
            self.tracker.idle.notify_waiters();
        }
    }
}

fn advance_epoch(state: &mut TrackerState) {
    match state.epoch.checked_add(1) {
        Some(epoch) => state.epoch = epoch,
        None => state.accepting = false,
    }
}

struct CancelOnDrop {
    cancellation: CancellationToken,
    completed: bool,
}

impl CancelOnDrop {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            completed: false,
        }
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
        }
    }
}
pub(super) fn errors_result(mut errors: Vec<anyhow::Error>) -> Result<()> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.remove(0)),
        _ => Err(anyhow::Error::new(OperationErrors(errors))),
    }
}

#[derive(Debug)]
struct OperationErrors(Vec<anyhow::Error>);

impl std::fmt::Display for OperationErrors {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} owned session operations failed",
            self.0.len()
        )?;
        for (index, error) in self.0.iter().enumerate() {
            write!(formatter, "; failure {}: {:#}", index + 1, error)?;
        }
        Ok(())
    }
}

impl std::error::Error for OperationErrors {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .first()
            .map(|error| error.as_ref() as &(dyn std::error::Error + 'static))
    }
}
fn record_state_poison(state: &mut TrackerState) {
    if !state.poison_reported {
        state.poison_reported = true;
        state
            .dropped_errors
            .push(SessionLifecycleFailure::OperationState.into());
    }
}

#[cfg(test)]
mod tests {
    use super::super::SessionLifecycleFailure;
    use super::OperationTracker;
    use anyhow::Result;

    #[test]
    fn multiple_retirement_failures_are_rendered_in_original_order() -> Result<()> {
        let markers = ["(A)", "(B)", "(C)"];
        let errors = vec![
            anyhow::Error::new(std::io::Error::other(markers[0])),
            anyhow::Error::new(std::io::Error::other(markers[1])),
            anyhow::Error::new(std::io::Error::other(markers[2])),
        ];
        let error = super::errors_result(errors)
            .err()
            .ok_or_else(|| anyhow::anyhow!("multiple failures must remain an error"))?;
        let aggregate = error
            .downcast_ref::<super::OperationErrors>()
            .ok_or_else(|| anyhow::anyhow!("multiple errors must retain their aggregate"))?;
        if aggregate.0.len() != markers.len()
            || !aggregate.0.iter().zip(markers).all(|(error, marker)| {
                error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|cause| cause.to_string() == marker)
            })
        {
            anyhow::bail!("retirement aggregate lost or reordered typed failures");
        }
        let report = format!("{error:#}");
        let positions = markers.map(|marker| report.find(marker));
        if !matches!(
            positions,
            [Some(first), Some(second), Some(third)] if first < second && second < third
        ) {
            anyhow::bail!("retirement report omitted or reordered original failures");
        }
        if !error
            .chain()
            .any(|cause| cause.downcast_ref::<std::io::Error>().is_some())
        {
            anyhow::bail!("retirement report lost the original typed cause");
        }
        Ok(())
    }

    #[tokio::test]
    async fn repeated_registry_poison_is_retained_once_until_retirement() -> Result<()> {
        let tracker = OperationTracker::new();
        tracker.record_owner_registry_poison();
        tracker.wait().await;
        let initial_count = tracker.state_lock()?.dropped_errors.len();
        tracker.record_owner_registry_poison();
        tracker.wait().await;
        let repeated_count = tracker.state_lock()?.dropped_errors.len();
        if initial_count != 1 || repeated_count != 1 {
            anyhow::bail!("repeated registry poison observation was retained more than once");
        }

        let errors = tracker.wait_and_collect_errors().await;
        if errors.len() != 1
            || errors[0].downcast_ref::<SessionLifecycleFailure>()
                != Some(&SessionLifecycleFailure::OwnerControlRegistry)
        {
            anyhow::bail!("retirement did not retain the one typed registry failure");
        }
        tracker.record_owner_registry_poison();
        if !tracker.state_lock()?.dropped_errors.is_empty() {
            anyhow::bail!("a retired registry poison was recorded a second time");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "session_search_operations_order_tests.rs"]
mod order_tests;
