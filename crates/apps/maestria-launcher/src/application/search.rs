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

pub(super) use passage_actions::activate_passage_action;
use passage_actions::{passage_actions, path_actions};
use passage_results::apply_passages;
pub(super) use passage_results::apply_result_filter;
pub(super) use passage_view::{
    close_passage_view, open_passage_view, passage_result_is_visible, path_result_is_visible,
};

fn reset_search_state(frontend: &Frontend, ui: &UiWeak, query: &str, has_search_service: bool) {
    {
        let mut model = lock(&frontend.model);
        model.query = query.to_string();
        model.pending_ticks = None;
        model.accepted.clear();
        model.accepted_passages.clear();
        model.accepted_paths.clear();
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
    let previous =
        frontend
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            });
    let generation = match previous {
        Ok(previous) => previous + 1,
        Err(_) => {
            show_notice(
                &ui,
                "Search generation exhausted; restart the launcher.".to_string(),
            );
            return;
        }
    };
    let search_service = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service());
    reset_search_state(&frontend, &ui, &query, search_service.is_some());

    runtime.spawn(async move {
        let response = state.search(query.clone(), generation).await;
        let apply_frontend = Arc::clone(&frontend);
        let apply_ui = ui.clone();
        let (applied, wait_for_application) = tokio::sync::oneshot::channel::<()>();
        let _ = slint::invoke_from_event_loop(move || {
            if apply_frontend.generation.load(Ordering::Acquire) == generation
                && let Some(window) = apply_ui.upgrade()
            {
                match response {
                    Ok(response) => apply_search_response(&window, &apply_frontend, response),
                    Err(error) => {
                        let mut model = lock(&apply_frontend.model);
                        model.accepted.clear();
                        model.accepted_passages.clear();
                        model.accepted_paths.clear();
                        model.displayed.clear();
                        drop(model);
                        window.set_results(empty_results());
                        window.set_actions(empty_actions());
                        window.set_status_kind("error".into());
                        window.set_status_message(error.message.into());
                    }
                }
            }
            drop(applied);
        });
        let _ = wait_for_application.await;
        if frontend.generation.load(Ordering::Acquire) != generation {
            return;
        }

        if let Some(config) = search_service
            && !query.is_empty()
        {
            let search_result = super::passages::search(config, &query).await;
            let merge_frontend = Arc::clone(&frontend);
            let merge_ui = ui.clone();
            let merge_query = query.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if merge_frontend.generation.load(Ordering::Acquire) != generation {
                    return;
                }
                if let Some(window) = merge_ui.upgrade() {
                    if let Some(search_result) = search_result {
                        apply_passages(
                            &window,
                            &merge_frontend,
                            generation,
                            &merge_query,
                            search_result,
                        );
                    } else {
                        window.set_index_status("Document search unavailable".into());
                    }
                }
            });
        }
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
