use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ModelRc, VecModel};

use super::window::show_notice;
use super::{
    ActionRow, DisplayedResult, Frontend, LauncherWindow, UiWeak, empty_actions, empty_results,
    lock,
};
use crate::ResultRow;
use crate::ipc::LauncherState;
use crate::model::{Action, ResultKind, SearchResponse, SearchResult, SearchStatusKind};

mod app_path_action;
pub(super) use app_path_action::activate_path_action;
mod passage_actions;
mod passage_results;
mod passage_view;
mod result_navigation;
mod typed_search;
pub(super) use typed_search::{
    advance_search_generation, apply_passage_result, finish_active_search, search_passages,
    start_catalog_search, start_typed_passage_search, start_typed_search,
    take_pending_passage_search,
};

pub(super) use passage_actions::activate_passage_action;
use passage_actions::{passage_actions, path_actions};
use passage_results::apply_passages;
pub(super) use passage_results::apply_refreshed_passages;
pub(super) use passage_results::apply_result_filter;
pub(super) use passage_view::{
    close_passage_view, open_passage_view, passage_result_is_visible, path_result_is_visible,
};
pub(super) use result_navigation::navigate_result_selection;

pub(super) fn reset_search_state(
    frontend: &Frontend,
    ui: &UiWeak,
    query: &str,
    has_search_service: bool,
) {
    {
        let mut model = lock(&frontend.model);
        model.query = query.to_string();
        model.passage_search_pending = false;
        model.accepted.clear();
        model.accepted_passages.clear();
        model.accepted_paths.clear();
        model.passages_loaded = false;
        model.displayed.clear();
        model.content_view_passages.clear();
        model.result_filter = "all".to_string();
        model.selected_file = None;
    }
    if let Some(window) = ui.upgrade() {
        window.set_results(empty_results());
        window.set_actions(empty_actions());
        window.set_passage_view_results(empty_results());
        window.set_passage_view_open(false);
        window.set_notice("".into());
        window.set_result_filter("all".into());
        window.set_index_status(
            if has_search_service && query.is_empty() {
                "Enter a query to search documents"
            } else if has_search_service {
                "Searching document index…"
            } else {
                "Document search not configured"
            }
            .into(),
        );
        window.set_selected_index(0);
        window.set_selected_action_index(0);
        window.set_actions_open(false);
        window.set_status_kind("loading".into());
        window.set_status_message("Searching…".into());
    }
}

pub(super) fn start_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
) {
    let Some(generation) = advance_search_generation(&frontend) else {
        show_notice(
            &ui,
            "Search generation exhausted; restart the launcher.".to_string(),
        );
        return;
    };
    frontend.active_search.store(generation, Ordering::Release);

    let (search_service, settings_error) = match state.settings() {
        Ok(settings) => (settings.search_service(), None),
        Err(error) => (None, Some(error.message)),
    };
    reset_search_state(&frontend, &ui, &query, search_service.is_some());
    if let Some(error) = settings_error {
        show_notice(&ui, error);
    }

    let catalog_applied = start_catalog_search(
        Arc::clone(&state),
        Arc::clone(&frontend),
        &runtime,
        ui.clone(),
        query.clone(),
        generation,
    );
    runtime.spawn(async move {
        let _ = catalog_applied.await;
        if frontend.generation.load(Ordering::Acquire) != generation {
            finish_active_search(&frontend, generation);
            return;
        }

        if search_service.is_some() && !query.is_empty() {
            let search_result = search_passages(&frontend, &state, generation, &query, &ui).await;
            if frontend.generation.load(Ordering::Acquire) == generation {
                let applied = apply_passage_result(
                    Arc::clone(&frontend),
                    ui,
                    generation,
                    query,
                    search_result,
                );
                let _ = applied.await;
            }
        }
        finish_active_search(&frontend, generation);
    });
}

fn apply_search_response(window: &LauncherWindow, frontend: &Frontend, response: SearchResponse) {
    let status_kind = match &response.status.kind {
        SearchStatusKind::Loading | SearchStatusKind::Refreshing => "loading",
        SearchStatusKind::Ready => "ready",
        SearchStatusKind::Error | SearchStatusKind::CalculationError => "error",
    };
    let mut message = String::new();
    if let Some(status_message) = &response.status.message {
        message.clone_from(status_message);
    }
    let rows = result_rows(&response.results);
    {
        let mut model = lock(&frontend.model);
        model.accepted.clone_from(&response.results);
        model.accepted_passages.clear();
        model.accepted_paths.clear();
        model.passages_loaded = false;
        model.content_view_passages.clear();
        model.result_filter = "all".to_string();
        model.displayed = (0..response.results.len())
            .map(DisplayedResult::Application)
            .collect();
        model.selected_file = None;
    }
    window.set_results(rows);
    window.set_result_filter("all".into());
    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.set_actions(actions_for_result(response.results.first()));
    window.set_selected_index(0);
    window.set_selected_action_index(0);
    window.set_actions_open(false);
    window.set_status_kind(status_kind.into());
    window.set_status_message(message.into());
}
fn apply_search_error(window: &LauncherWindow, frontend: &Frontend, message: String) {
    let mut model = lock(&frontend.model);
    model.accepted.clear();
    model.accepted_passages.clear();
    model.accepted_paths.clear();
    model.passages_loaded = false;
    model.displayed.clear();
    drop(model);
    window.set_results(empty_results());
    window.set_actions(empty_actions());
    window.set_status_kind("error".into());
    window.set_status_message(message.into());
}

pub(super) fn update_selected_actions(window: &LauncherWindow, frontend: &Frontend, index: usize) {
    let model = lock(&frontend.model);
    if model.selected_file.is_some() {
        window.set_actions(file_actions());
        window.set_selected_action_index(0);
        return;
    }
    let actions = match model.displayed.get(index) {
        Some(DisplayedResult::Application(result_index)) => {
            actions_for_result(model.accepted.get(*result_index))
        }
        Some(DisplayedResult::Passage(passage_index)) => model
            .accepted_passages
            .get(*passage_index)
            .map_or_else(empty_actions, |_| passage_actions()),
        Some(DisplayedResult::Path(path_index)) => model
            .accepted_paths
            .get(*path_index)
            .map_or_else(empty_actions, |_| path_actions()),
        Some(DisplayedResult::Group) | None => empty_actions(),
    };
    window.set_actions(actions);
    window.set_selected_action_index(0);
}

fn result_rows(results: &[SearchResult]) -> ModelRc<ResultRow> {
    ModelRc::new(VecModel::from(
        results.iter().map(result_row).collect::<Vec<_>>(),
    ))
}

fn result_row(result: &SearchResult) -> ResultRow {
    ResultRow {
        id: result.id.clone().into(),
        title: result.title.clone().into(),
        subtitle: result.subtitle.clone().into(),
        kind: result_kind_label(&result.kind).into(),
        accessible_name: format!("{} {}", result.title, result.subtitle).into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: "".into(),
    }
}

fn actions_for_result(result: Option<&SearchResult>) -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(result.map_or_else(Vec::new, |result| {
        result.actions.iter().map(action_row).collect::<Vec<_>>()
    })))
}

fn action_row(action: &Action) -> ActionRow {
    ActionRow {
        id: action.id.clone().into(),
        title: action.title.clone().into(),
        accessible_name: format!("{} action", action.title).into(),
    }
}

pub(super) fn file_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(vec![
        ActionRow {
            id: "file.open".into(),
            title: "Open".into(),
            accessible_name: "Open selected file".into(),
        },
        ActionRow {
            id: "file.copy-path".into(),
            title: "Copy Path".into(),
            accessible_name: "Copy selected file path".into(),
        },
    ]))
}

pub(super) fn primary_action(result: &SearchResult) -> Option<&str> {
    result
        .actions
        .iter()
        .find(|action| action.primary)
        .map(|action| action.id.as_str())
}

fn result_kind_label(kind: &ResultKind) -> &'static str {
    match kind {
        ResultKind::Application => "application",
        ResultKind::Command => "command",
        ResultKind::Calculation => "calculation",
    }
}
