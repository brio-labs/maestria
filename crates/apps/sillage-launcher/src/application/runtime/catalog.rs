use std::sync::Arc;

use slint::{ComponentHandle, Model};

use super::TimerState;
use crate::application::search::start_search;
use crate::application::{CATALOG_REFRESH_TICKS, Frontend, LauncherWindow, lock};
use crate::ipc::LauncherState;

pub(super) fn refresh_catalog_if_due(
    ui: &LauncherWindow,
    state: &LauncherState,
    frontend: &Frontend,
) {
    let due = {
        let mut model = lock(&frontend.model);
        model.catalog_ticks_until_refresh = model.catalog_ticks_until_refresh.saturating_sub(1);
        model.catalog_ticks_until_refresh == 0
    };
    if due && ui.window().is_visible() && state.catalog().request_refresh().is_ok() {
        lock(&frontend.model).catalog_ticks_until_refresh = CATALOG_REFRESH_TICKS;
    }
}

pub(super) fn update_catalog_status(
    timer: &mut TimerState,
    ui: &LauncherWindow,
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
) {
    let Ok(snapshot) = state.catalog().snapshot() else {
        return;
    };
    let ready = matches!(
        &snapshot.status.kind,
        crate::model::SearchStatusKind::Ready | crate::model::SearchStatusKind::Error
    );
    if ready && snapshot.revision != timer.last_catalog_revision {
        // Preserve an open evidence view; the pending revision refreshes once
        // the user returns to results instead of invalidating its Copy controls.
        if ui.get_passage_view_open() || ui.get_utilities_open() || ui.get_actions_open() {
            return;
        }
        timer.last_catalog_revision = snapshot.revision;
        let query = lock(&frontend.model).query.clone();
        let selected = ui
            .get_results()
            .row_data(ui.get_selected_index().max(0) as usize)
            .map(|row| row.id);
        start_search(
            state,
            frontend,
            runtime.clone(),
            ui.as_weak(),
            query,
            selected,
        );
    } else if matches!(
        &snapshot.status.kind,
        crate::model::SearchStatusKind::Loading | crate::model::SearchStatusKind::Refreshing
    ) {
        ui.set_status_kind("loading".into());
        let message = snapshot.status.message.as_deref().map_or_else(
            || "Refreshing installed applications…".to_string(),
            str::to_owned,
        );
        ui.set_status_message(message.into());
    }
}
