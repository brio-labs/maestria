use std::sync::{Arc, mpsc};

use slint::ComponentHandle;

use super::search::start_search;
use super::{Frontend, LauncherWindow, SOURCE_REFRESH_TICKS, lock};
use crate::ipc::LauncherState;

/// Poll only the authorized, indexed source-event clock. Search is restarted
/// at most once per observed change, never on a timer or an unchanged result.
pub(super) struct SourceRefresh {
    ticks_until_poll: u8,
    in_flight: bool,
    last_revision: Option<i64>,
    changed: bool,
    sender: mpsc::SyncSender<Option<i64>>,
    receiver: mpsc::Receiver<Option<i64>>,
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
        if !ui.window().is_visible() {
            return;
        }

        let query = {
            let model = lock(&frontend.model);
            self.should_refresh(
                &model.query,
                model.pending_ticks.is_some(),
                ui.get_passage_view_open(),
            )
            .then(|| model.query.clone())
        };
        if let Some(query) = query {
            start_search(
                Arc::clone(state),
                Arc::clone(frontend),
                runtime.clone(),
                ui.as_weak(),
                query,
            );
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
