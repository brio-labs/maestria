use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use slint::ComponentHandle;

use super::search::apply_refreshed_passages;
use super::{Frontend, LauncherWindow, SOURCE_REFRESH_TICKS, lock};
use crate::ipc::LauncherState;

/// Poll only the authorized, indexed source-event clock. Refresh just the
/// passages, leaving accepted actions and visible rows intact until a changed
/// result arrives. Never reissue a query for an unchanged clock.
pub(super) struct SourceRefresh {
    ticks_until_poll: u8,
    in_flight: bool,
    last_revision: Option<i64>,
    changed: bool,
    sender: mpsc::SyncSender<Option<i64>>,
    receiver: mpsc::Receiver<Option<i64>>,
    deferred: Arc<AtomicBool>,
    latest_refresh: Arc<AtomicU64>,
}

impl Default for SourceRefresh {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        Self {
            ticks_until_poll: 0,
            in_flight: false,
            last_revision: None,
            changed: false,
            sender,
            receiver,
            deferred: Arc::new(AtomicBool::new(false)),
            latest_refresh: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl SourceRefresh {
    pub(super) fn tick(
        &mut self,
        ui: &LauncherWindow,
        state: &Arc<LauncherState>,
        frontend: &Arc<Frontend>,
        runtime: &tokio::runtime::Handle,
    ) {
        while let Ok(revision) = self.receiver.try_recv() {
            self.in_flight = false;
            if let Some(revision) = revision {
                self.observe(revision);
            }
        }
        if self.deferred.swap(false, Ordering::AcqRel) {
            self.changed = true;
        }
        if !ui.window().is_visible() {
            return;
        }

        let query = {
            let model = lock(&frontend.model);
            // Leave the source revision pending until the foreground query applies.
            if frontend.active_search.load(Ordering::Acquire) != 0 {
                None
            } else {
                self.should_refresh(
                    &model.query,
                    model.pending_ticks.is_some(),
                    ui.get_passage_view_open() || model.selected_file.is_some(),
                )
                .then(|| model.query.clone())
            }
        };
        if let Some(query) = query {
            self.refresh_passages(ui, state, frontend, runtime, query);
        }

        if self.ticks_until_poll > 0 {
            self.ticks_until_poll -= 1;
            return;
        }
        self.ticks_until_poll = SOURCE_REFRESH_TICKS;
        if self.in_flight {
            return;
        }
        let Some(config) = state
            .settings()
            .ok()
            .and_then(|settings| settings.search_service())
        else {
            return;
        };
        self.in_flight = true;
        let sender = self.sender.clone();
        runtime.spawn(async move {
            let revision = super::passages::source_revision(config).await;
            let _ = sender.try_send(revision);
        });
    }

    fn refresh_passages(
        &self,
        ui: &LauncherWindow,
        state: &Arc<LauncherState>,
        frontend: &Arc<Frontend>,
        runtime: &tokio::runtime::Handle,
        query: String,
    ) {
        if state
            .settings()
            .ok()
            .and_then(|settings| settings.search_service())
            .is_none()
        {
            return;
        }
        let generation = frontend.generation.load(Ordering::Acquire);
        let serial = self
            .latest_refresh
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let latest_refresh = Arc::clone(&self.latest_refresh);
        let deferred = Arc::clone(&self.deferred);
        let frontend = Arc::clone(frontend);
        let state = Arc::clone(state);
        let ui = ui.as_weak();
        runtime.spawn(async move {
            let result =
                super::search::search_passages(&frontend, &state, generation, &query).await;
            let _ = slint::invoke_from_event_loop(move || {
                if latest_refresh.load(Ordering::Acquire) != serial
                    || frontend.generation.load(Ordering::Acquire) != generation
                {
                    return;
                }
                let Some(window) = ui.upgrade() else {
                    return;
                };
                let model = lock(&frontend.model);
                if model.query != query {
                    return;
                }
                if !window.window().is_visible()
                    || window.get_passage_view_open()
                    || model.pending_ticks.is_some()
                    || model.selected_file.is_some()
                {
                    deferred.store(true, Ordering::Release);
                    return;
                }
                drop(model);
                if let Some(result) = result {
                    apply_refreshed_passages(&window, &frontend, generation, &query, result);
                } else {
                    window.set_index_status("Document search unavailable".into());
                }
            });
        });
    }

    fn observe(&mut self, revision: i64) {
        if self
            .last_revision
            .replace(revision)
            .is_some_and(|last| last != revision)
        {
            self.changed = true;
        }
    }

    fn should_refresh(&mut self, query: &str, debouncing: bool, detail_open: bool) -> bool {
        if !self.changed || debouncing || detail_open {
            return false;
        }
        self.changed = false;
        !query.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::SourceRefresh;

    #[test]
    fn refreshes_visible_query_once_per_real_index_change_after_detail_and_debounce() {
        let mut refresh = SourceRefresh::default();
        refresh.observe(4);
        assert!(!refresh.should_refresh("fresh", false, false));
        refresh.observe(4);
        assert!(!refresh.should_refresh("fresh", false, false));

        refresh.observe(5);
        assert!(!refresh.should_refresh("fresh", false, true));
        assert!(!refresh.should_refresh("fresh", true, false));
        assert!(refresh.should_refresh("fresh", false, false));
        assert!(!refresh.should_refresh("fresh", false, false));

        refresh.observe(6);
        assert!(!refresh.should_refresh("", false, false));
        assert!(!refresh.should_refresh("fresh", false, false));
    }
}
