use std::sync::Arc;
use std::sync::atomic::Ordering;

use sillage_domain::{ControlPersistenceChoice, DocumentaryRetentionChoice};

use super::super::state::SessionSourceState;
use super::SourceChoiceController;

impl SourceChoiceController {
    pub(super) fn publish(&self) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        let snapshot = self.state.snapshot();
        let ui = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = ui.upgrade() {
                let selected_path = match snapshot
                    .selected_path
                    .as_ref()
                    .and_then(|path| path.to_str())
                {
                    Some(path) => path.into(),
                    None => "".into(),
                };
                window.set_source_selected_path(selected_path);
                window.set_source_read_consent(snapshot.consent);
                window.set_source_documentary_retention(
                    snapshot.documentary_retention
                        == DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
                );
                window.set_source_control_persistence(
                    snapshot.control_persistence == ControlPersistenceChoice::PersistExplicitly,
                );
                window.set_source_phase(snapshot.phase.label().into());
                window.set_source_status(snapshot.status.into());
                window.set_source_detail(snapshot.detail.into());
                window.set_source_error(snapshot.error.into());
                window.set_source_busy(snapshot.busy);
                window.set_source_prepare_enabled(snapshot.can_prepare);
                window.set_source_search_enabled(snapshot.can_search);
                window.set_source_purge_enabled(snapshot.can_purge);
            }
        });
    }
}

pub(super) async fn run_state_publisher(
    state: Arc<SessionSourceState>,
    mut is_stopping: impl FnMut() -> bool + Send,
    mut publish: impl FnMut() + Send,
) {
    let mut changes = state.subscribe_changes();
    loop {
        if is_stopping() {
            return;
        }
        if changes.changed().await.is_err() || is_stopping() {
            return;
        }
        publish();
    }
}

#[cfg(test)]
mod publisher_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::run_state_publisher;
    use crate::application::source_choice::state::SessionSourceState;
    use tokio::time::timeout;

    #[tokio::test]
    async fn state_publisher_exits_when_started_after_shutdown_notification() -> Result<(), String>
    {
        let state = Arc::new(SessionSourceState::default());
        state.stop_new_admission();
        let stopping = Arc::new(AtomicBool::new(true));
        match timeout(
            Duration::from_millis(50),
            run_state_publisher(state, move || stopping.load(Ordering::Acquire), || {}),
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(_) => {
                Err("publisher must notice shutdown before waiting for another update".to_string())
            }
        }
    }
}
