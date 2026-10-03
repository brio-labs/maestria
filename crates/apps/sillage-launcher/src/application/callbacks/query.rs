use std::sync::Arc;

use slint::{ComponentHandle, Timer, TimerMode};

use super::super::search::{
    navigate_result_selection, start_typed_passage_search, start_typed_search,
    take_pending_passage_search, update_selected_actions,
};
use super::super::window::show_notice;
use super::super::{
    Frontend, LauncherWindow, PASSAGE_SEARCH_DEBOUNCE, empty_actions, empty_results, lock,
};
use crate::ipc::LauncherState;

pub(super) fn install_query_callbacks(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
) {
    install_query_changed_callback(ui, state, frontend, runtime);
    install_result_selection_callbacks(ui, frontend);
}

fn install_query_changed_callback(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
) {
    let weak = ui.as_weak();
    let query_frontend = Arc::clone(frontend);
    let query_state = Arc::clone(state);
    let query_runtime = runtime.clone();
    // This callback owns the timer for the lifetime of the launcher window.
    let passage_search_timer = Timer::default();
    ui.on_query_changed(move |query| {
        let query = query.to_string();
        passage_search_timer.stop();
        let pending_search = start_typed_search(
            Arc::clone(&query_state),
            Arc::clone(&query_frontend),
            query_runtime.clone(),
            weak.clone(),
            query.clone(),
        );
        let has_search_service = pending_search
            .as_ref()
            .is_some_and(|search| search.has_search_service);
        {
            let mut model = lock(&query_frontend.model);
            model.query.clone_from(&query);
            model.passage_search_pending = pending_search.is_some();
            model.accepted.clear();
            model.accepted_passages.clear();
            model.accepted_paths.clear();
            model.passages_loaded = false;
            model.displayed.clear();
            model.content_view_passages.clear();
            model.result_filter = "all".to_string();
            model.selected_file = None;
        }
        if let Some(ui) = weak.upgrade() {
            ui.set_results(empty_results());
            ui.set_actions(empty_actions());
            ui.set_actions_open(false);
            ui.set_selected_action_index(0);
            ui.set_selected_index(0);
            ui.set_passage_view_open(false);
            ui.set_passage_view_results(empty_results());
            ui.set_result_filter("all".into());
            ui.set_index_status(
                if has_search_service && query.is_empty() {
                    "Enter a query to search documents"
                } else if has_search_service {
                    "Searching document index…"
                } else {
                    "Document search not configured"
                }
                .into(),
            );
            ui.set_status_kind("loading".into());
            ui.set_status_message("Searching…".into());
        } else {
            lock(&query_frontend.model).passage_search_pending = false;
            return;
        }
        let Some(pending_search) = pending_search else {
            show_notice(
                &weak,
                "Search generation exhausted; restart the launcher.".to_string(),
            );
            return;
        };
        let generation = pending_search.generation;
        let timer_frontend = Arc::clone(&query_frontend);
        let timer_state = Arc::clone(&query_state);
        let timer_runtime = query_runtime.clone();
        let timer_ui = weak.clone();
        let mut catalog_applied = Some(pending_search.catalog_applied);
        passage_search_timer.start(TimerMode::SingleShot, PASSAGE_SEARCH_DEBOUNCE, move || {
            let Some(catalog_applied) = catalog_applied.take() else {
                return;
            };
            let Some((pending_query, pending_generation)) =
                take_pending_passage_search(&timer_frontend)
            else {
                return;
            };
            if pending_generation != generation || !has_search_service || pending_query.is_empty() {
                return;
            }
            start_typed_passage_search(
                Arc::clone(&timer_state),
                Arc::clone(&timer_frontend),
                timer_runtime.clone(),
                timer_ui.clone(),
                pending_query,
                pending_generation,
                catalog_applied,
            );
        });
    });
}

fn install_result_selection_callbacks(ui: &LauncherWindow, frontend: &Arc<Frontend>) {
    let weak = ui.as_weak();
    let selection_frontend = Arc::clone(frontend);
    ui.on_selection_changed(move |index| {
        if let Some(ui) = weak.upgrade() {
            update_selected_actions(&ui, &selection_frontend, index.max(0) as usize);
        }
    });

    let weak = ui.as_weak();
    let navigation_frontend = Arc::clone(frontend);
    ui.on_result_navigation_requested(move |forward| {
        if let Some(ui) = weak.upgrade() {
            navigate_result_selection(&ui, &navigation_frontend, forward);
        }
    });
}
