use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{
    SessionSearchCancellation, SessionSourceState, SessionState, SourcePhase, SourceQuery,
    TerminalOperationReservation, lock,
};

fn settle_terminal_reservation(
    state: &mut SessionState,
    reservation: &mut TerminalOperationReservation<'_>,
    error: Option<String>,
) {
    state.reserved_terminal_operation_slots -= 1;
    if let Some(error) = error {
        state.terminal_operation_failures.push(error);
    }
    reservation.active = false;
}

impl SessionSourceState {
    pub(in crate::application::source_choice) async fn wait_for_operations(&self) {
        loop {
            let notified = self.operations_changed.notified();
            let running = {
                let state = lock(&self.inner);
                state.query_running || state.proof_running
            };
            if !running {
                return;
            }
            notified.await;
        }
    }

    pub(in crate::application::source_choice) fn cancel_operations(&self) {
        let mut state = lock(&self.inner);
        if let Some(cancellation) = state.prepare_cancellation.take() {
            cancellation.cancel();
        }
        if let Some(cancellation) = state.query_cancellation.take() {
            cancellation.cancel();
        }
        if let Some(cancellation) = state.proof_cancellation.take() {
            cancellation.cancel();
        }
        drop(state);
        self.queries.send_replace(None);
        self.operations_changed.notify_waiters();
    }

    pub(in crate::application::source_choice) fn set_retiring(&self) {
        let mut state = lock(&self.inner);
        state.epoch = state.epoch.saturating_add(1);
        state.current_request_id = None;
        state.source_revision = None;
        state.phase = SourcePhase::Retiring;
        state.status = "Retiring previous source; waiting for owned work".to_string();
        state.detail.clear();
        drop(state);
        self.notify_changed();
    }

    pub(in crate::application::source_choice) async fn retire_active(&self) -> Result<(), String> {
        self.cancel_operations();
        self.wait_for_operations().await;
        let active = {
            let state = lock(&self.inner);
            state
                .active
                .as_ref()
                .map(|active| (Arc::clone(&active.owner), Arc::clone(&active.runtime)))
        };
        let Some((owner, runtime)) = active else {
            return Ok(());
        };
        let runtime_result = runtime.retire().await;
        runtime.wait_for_idle().await;
        let owner_result = owner.retire().await;
        owner.wait_for_idle().await;
        match (runtime_result, owner_result) {
            (Ok(()), Ok(())) => {
                let mut state = lock(&self.inner);
                if state.active.as_ref().is_some_and(|active| {
                    Arc::ptr_eq(&active.owner, &owner) && Arc::ptr_eq(&active.runtime, &runtime)
                }) {
                    state.active.take();
                }
                Ok(())
            }
            (runtime, owner) => Err(format!(
                "runtime settlement: {}; preparation-owner settlement: {}",
                runtime.map_or_else(|error| format!("{error:#}"), |_| "settled".to_string()),
                owner.map_or_else(|error| format!("{error:#}"), |_| "settled".to_string()),
            )),
        }
    }

    pub(in crate::application::source_choice) fn clear_selection(&self) {
        let mut state = lock(&self.inner);
        state.selected_path = None;
        state.consent = false;
        state.active = None;
        state.prepare_cancellation = None;
        state.query_cancellation = None;
        state.proof_cancellation = None;
        state.current_request_id = None;
        state.source_revision = None;
        state.phase = SourcePhase::Unselected;
        state.status = "No Markdown file selected".to_string();
        state.detail = "Selection is required before source preparation.".to_string();
        state.error.clear();
        drop(state);
        self.notify_changed();
    }

    pub(in crate::application::source_choice) fn stop_new_admission(&self) {
        self.stopping.store(true, Ordering::Release);
        self.cancel_operations();
        let mut state = lock(&self.inner);
        state.phase = SourcePhase::Retiring;
        state.status = "Stopping selected-source admission".to_string();
        drop(state);
        self.notify_changed();
    }

    pub(in crate::application::source_choice) fn finish_retirement(&self) {
        let mut state = lock(&self.inner);
        state.active = None;
        state.consent = false;
        state.phase = if state.selected_path.is_some() {
            SourcePhase::Selected
        } else {
            SourcePhase::Unselected
        };
        state.status = if state.selected_path.is_some() {
            "Selected Markdown file is not prepared; press Prepare before searching".to_string()
        } else {
            "No Markdown file selected".to_string()
        };
        state.detail = "Reading requires explicit consent and a fresh Prepare gesture.".to_string();
        state.source_revision = None;
        drop(state);
        self.notify_changed();
    }

    pub(in crate::application::source_choice) fn mark_stopped(&self) {
        let mut state = lock(&self.inner);
        state.phase = SourcePhase::Stopped;
        state.status = "Selected-source session stopped".to_string();
        state.active = None;
        drop(state);
        self.notify_changed();
    }

    pub(in crate::application::source_choice) fn settle_prepare_failure(
        &self,
        epoch: u64,
        cancellation: &SessionSearchCancellation,
        error: String,
        routine_settlement: bool,
        retain_critical_failure: bool,
        mut terminal_slot: TerminalOperationReservation<'_>,
    ) {
        let mut state = lock(&self.inner);
        let current = !self.stopping.load(Ordering::Acquire) && state.epoch == epoch;
        let cancelled = cancellation.is_cancelled();
        let retain = retain_critical_failure || (!routine_settlement && (!current || cancelled));
        settle_terminal_reservation(
            &mut state,
            &mut terminal_slot,
            retain.then(|| error.clone()),
        );
        if current {
            state.prepare_cancellation = None;
            state.phase = if cancelled {
                SourcePhase::Cancelled
            } else {
                SourcePhase::Error
            };
            state.status = if cancelled {
                "Preparation cancelled".to_string()
            } else {
                "Preparation failed".to_string()
            };
            state.error = error;
        }
        drop(state);
        drop(terminal_slot);
        if current {
            self.notify_changed();
        }
    }

    pub(in crate::application::source_choice) fn settle_query_failure(
        &self,
        request: &SourceQuery,
        error: String,
        refused: bool,
        routine_settlement: bool,
        mut terminal_slot: TerminalOperationReservation<'_>,
    ) -> bool {
        let mut state = lock(&self.inner);
        let matches_request = state.epoch == request.epoch
            && state.query_generation == request.generation
            && state.current_request_id == Some(request.request_id);
        let stopping = self.stopping.load(Ordering::Acquire);
        let cancelled = request.cancellation.is_cancelled() || stopping;
        let publish = matches_request && !cancelled;
        let retain = !routine_settlement && (!matches_request || cancelled);
        settle_terminal_reservation(
            &mut state,
            &mut terminal_slot,
            retain.then(|| error.clone()),
        );
        if matches_request {
            state.query_cancellation = None;
            if !cancelled {
                state.phase = if refused {
                    SourcePhase::Refused
                } else {
                    SourcePhase::Error
                };
                state.status = state.phase.label().to_string();
                state.error = error;
            }
        }
        drop(state);
        drop(terminal_slot);
        if publish {
            self.notify_changed();
        }
        publish
    }

    pub(in crate::application::source_choice) fn set_error(&self, message: String) {
        let mut state = lock(&self.inner);
        if !self.stopping.load(Ordering::Acquire)
            && !matches!(
                state.phase,
                SourcePhase::Preparing | SourcePhase::Retiring | SourcePhase::Stopped
            )
        {
            state.phase = SourcePhase::Error;
        }
        state.status = "Selected-source operation failed".to_string();
        state.error = message;
        drop(state);
        self.notify_changed();
    }
}
