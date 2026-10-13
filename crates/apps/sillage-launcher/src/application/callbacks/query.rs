use std::sync::Arc;

use slint::{ComponentHandle, Timer, TimerMode};
use tokio::runtime::Handle;

use super::super::search::{
    TypedSearch, navigate_result_selection, start_typed_passage_search, start_typed_search,
    take_pending_passage_search, update_selected_actions,
};
use super::super::window::show_notice;
use super::super::{
    Frontend, LauncherWindow, PASSAGE_SEARCH_DEBOUNCE, UiWeak, empty_actions, empty_results, lock,
};
use crate::ipc::LauncherState;

pub(super) fn install_query_callbacks(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    runtime: &Handle,
) {
    install_query_changed_callback(ui, state, frontend, runtime);
    install_result_selection_callbacks(ui, frontend);
}

fn install_query_changed_callback(
    ui: &LauncherWindow,
    state: &Arc<LauncherState>,
    frontend: &Arc<Frontend>,
    runtime: &Handle,
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
        let source_route = pending_search
            .as_ref()
            .is_some_and(|search| search.source_route);
        reset_query_model(&query_frontend, &query, pending_search.is_some());
        if !reset_query_ui(
            &weak,
            &query_frontend,
            &query,
            source_route,
            has_search_service,
        ) {
            return;
        }
        let Some(pending_search) = pending_search else {
            show_notice(
                &weak,
                "Search generation exhausted; restart the launcher.".to_string(),
            );
            return;
        };
        schedule_passage_search(
            &passage_search_timer,
            pending_search,
            Arc::clone(&query_frontend),
            Arc::clone(&query_state),
            query_runtime.clone(),
            weak.clone(),
        );
    });
}

fn reset_query_model(frontend: &Frontend, query: &str, pending: bool) {
    let mut model = lock(&frontend.model);
    model.query.clear();
    model.query.push_str(query);
    model.passage_search_pending = pending;
    model.accepted.clear();
    model.accepted_passages.clear();
    model.accepted_paths.clear();
    model.passages_loaded = false;
    model.displayed.clear();
    model.content_view_passages.clear();
    model.result_filter = "all".to_string();
    model.selected_file = None;
}

fn reset_query_ui(
    weak: &UiWeak,
    frontend: &Frontend,
    query: &str,
    source_route: bool,
    has_search_service: bool,
) -> bool {
    let Some(ui) = weak.upgrade() else {
        lock(&frontend.model).passage_search_pending = false;
        return false;
    };
    ui.set_results(empty_results());
    ui.set_actions(empty_actions());
    ui.set_actions_open(false);
    ui.set_selected_action_index(0);
    ui.set_selected_index(0);
    ui.set_passage_view_open(false);
    ui.set_passage_view_results(empty_results());
    ui.set_result_filter("all".into());
    let (index_status, status_kind, status_message) = if source_route {
        (
            "Selected Markdown source; no folder fallback".to_string(),
            if query.is_empty() { "ready" } else { "loading" }.to_string(),
            if query.is_empty() {
                "Enter a query to search the selected Markdown file.".to_string()
            } else {
                "Search is restricted to the selected Markdown file.".to_string()
            },
        )
    } else if has_search_service && query.is_empty() {
        (
            "Enter a query to search documents".to_string(),
            "loading".to_string(),
            "Searching…".to_string(),
        )
    } else if has_search_service {
        (
            "Searching document index…".to_string(),
            "loading".to_string(),
            "Searching…".to_string(),
        )
    } else {
        (
            "Document search not configured".to_string(),
            "ready".to_string(),
            "Configure document search to search folders.".to_string(),
        )
    };
    ui.set_index_status(index_status.into());
    ui.set_status_kind(status_kind.into());
    ui.set_status_message(status_message.into());
    true
}

fn schedule_passage_search(
    timer: &Timer,
    pending_search: TypedSearch,
    frontend: Arc<Frontend>,
    state: Arc<LauncherState>,
    runtime: Handle,
    ui: UiWeak,
) {
    let TypedSearch {
        generation,
        has_search_service,
        source_route,
        source_ready,
        catalog_applied,
    } = pending_search;
    let mut catalog_applied = Some(catalog_applied);
    let mut source_ready = source_ready;
    timer.start(TimerMode::SingleShot, PASSAGE_SEARCH_DEBOUNCE, move || {
        let Some(catalog_applied) = catalog_applied.take() else {
            return;
        };
        let Some((pending_query, pending_generation)) = take_pending_passage_search(&frontend)
        else {
            return;
        };
        if pending_generation != generation || pending_query.is_empty() {
            return;
        }
        if source_route {
            let Some(ready) = source_ready.take() else {
                return;
            };
            if let Err(error) = frontend.source_choice.submit_query(
                pending_query,
                pending_generation,
                catalog_applied,
            ) {
                show_notice(&ui, error);
            } else {
                let _ = ready.send(());
            }
            return;
        }
        if !has_search_service {
            return;
        }
        start_typed_passage_search(
            Arc::clone(&state),
            Arc::clone(&frontend),
            runtime.clone(),
            ui.clone(),
            pending_query,
            pending_generation,
            catalog_applied,
        );
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
