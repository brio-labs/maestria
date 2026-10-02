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

pub(super) use passage_actions::activate_passage_action;
use passage_actions::{passage_actions, path_actions};
use passage_results::apply_passages;
pub(super) use passage_results::apply_refreshed_passages;
pub(super) use passage_results::apply_result_filter;
pub(super) use passage_view::{
    close_passage_view, open_passage_view, passage_result_is_visible, path_result_is_visible,
};
pub(super) use result_navigation::navigate_result_selection;

fn reset_search_state(frontend: &Frontend, ui: &UiWeak, query: &str, has_search_service: bool) {
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

/// Serialize shared-realm searches and cancel requests when a newer UI search starts.
pub(super) async fn search_passages(
    frontend: &Frontend,
    state: &LauncherState,
    generation: u64,
    query: &str,
) -> Option<super::passages::PassageSearchResult> {
    let mut generation_updates = frontend.generation_updates.subscribe();
    let _interactive_search =
        acquire_interactive_search_slot(frontend, generation, &mut generation_updates).await?;

    // Resolve the authorized service after waiting for the shared request slot.
    let config = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service())?;
    tokio::select! {
        biased;
        _ = generation_updates.changed() => None,
        result = super::passages::search(config, query) => result,
    }
}

async fn acquire_interactive_search_slot<'a>(
    frontend: &'a Frontend,
    generation: u64,
    generation_updates: &mut tokio::sync::watch::Receiver<u64>,
) -> Option<tokio::sync::MutexGuard<'a, ()>> {
    let guard = tokio::select! {
        biased;
        _ = generation_updates.changed() => return None,
        guard = frontend.interactive_search.lock() => guard,
    };
    (frontend.generation.load(Ordering::Acquire) == generation).then_some(guard)
}

pub(super) struct TypedSearch {
    pub(super) generation: u64,
    pub(super) has_search_service: bool,
    pub(super) catalog_applied: tokio::sync::oneshot::Receiver<()>,
}

pub(super) fn advance_search_generation(frontend: &Frontend) -> Option<u64> {
    let previous = frontend
        .generation
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
            generation.checked_add(1)
        })
        .ok()?;
    let generation = previous + 1;
    frontend.generation_updates.send_replace(generation);
    Some(generation)
}

pub(super) fn start_typed_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
) -> Option<TypedSearch> {
    frontend.active_search.store(0, Ordering::Release);
    let generation = advance_search_generation(&frontend)?;
    let has_search_service = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service())
        .is_some();
    let catalog_applied = start_catalog_search(
        state,
        Arc::clone(&frontend),
        &runtime,
        ui,
        query,
        generation,
    );
    Some(TypedSearch {
        generation,
        has_search_service,
        catalog_applied,
    })
}

pub(super) fn take_pending_passage_search(frontend: &Frontend) -> Option<(String, u64)> {
    let mut model = lock(&frontend.model);
    if !model.passage_search_pending {
        return None;
    }
    model.passage_search_pending = false;
    Some((
        model.query.clone(),
        frontend.generation.load(Ordering::Acquire),
    ))
}

pub(super) fn start_typed_passage_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
    generation: u64,
    catalog_applied: tokio::sync::oneshot::Receiver<()>,
) {
    if query.is_empty() {
        finish_active_search(&frontend, generation);
        return;
    }
    frontend.active_search.store(generation, Ordering::Release);
    runtime.spawn(async move {
        let mut generation_updates = frontend.generation_updates.subscribe();
        let search_result = search_passages(&frontend, &state, generation, &query).await;
        if frontend.generation.load(Ordering::Acquire) != generation {
            finish_active_search(&frontend, generation);
            return;
        }
        // The catalog apply resets passage rows, so only the RPC may run in parallel.
        if !wait_for_catalog_application(
            &frontend,
            generation,
            &mut generation_updates,
            catalog_applied,
        )
        .await
        {
            finish_active_search(&frontend, generation);
            return;
        }
        if frontend.generation.load(Ordering::Acquire) != generation {
            finish_active_search(&frontend, generation);
            return;
        }
        let applied =
            apply_passage_result(Arc::clone(&frontend), ui, generation, query, search_result);
        let _ = applied.await;
        finish_active_search(&frontend, generation);
    });
}

async fn wait_for_catalog_application(
    frontend: &Frontend,
    generation: u64,
    generation_updates: &mut tokio::sync::watch::Receiver<u64>,
    catalog_applied: tokio::sync::oneshot::Receiver<()>,
) -> bool {
    tokio::select! {
        biased;
        _ = generation_updates.changed() => false,
        result = catalog_applied => {
            result.is_ok() && frontend.generation.load(Ordering::Acquire) == generation
        },
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

    let search_service = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service());
    reset_search_state(&frontend, &ui, &query, search_service.is_some());

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
            let search_result = search_passages(&frontend, &state, generation, &query).await;
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

fn start_catalog_search(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: &tokio::runtime::Handle,
    ui: UiWeak,
    query: String,
    generation: u64,
) -> tokio::sync::oneshot::Receiver<()> {
    let (applied, wait_for_application) = tokio::sync::oneshot::channel::<()>();
    runtime.spawn(async move {
        let response = state.search(query, generation).await;
        let _ = slint::invoke_from_event_loop(move || {
            if frontend.generation.load(Ordering::Acquire) == generation
                && let Some(window) = ui.upgrade()
            {
                match response {
                    Ok(response) => apply_search_response(&window, &frontend, response),
                    Err(error) => apply_search_error(&window, &frontend, error.message),
                }
                let _ = applied.send(());
            }
        });
    });
    wait_for_application
}

fn apply_passage_result(
    frontend: Arc<Frontend>,
    ui: UiWeak,
    generation: u64,
    query: String,
    search_result: Option<super::passages::PassageSearchResult>,
) -> tokio::sync::oneshot::Receiver<()> {
    let (applied, wait_for_application) = tokio::sync::oneshot::channel::<()>();
    let _ = slint::invoke_from_event_loop(move || {
        if frontend.generation.load(Ordering::Acquire) == generation
            && let Some(window) = ui.upgrade()
        {
            if let Some(search_result) = search_result {
                apply_passages(&window, &frontend, generation, &query, search_result, false);
            } else {
                window.set_index_status("Document search unavailable".into());
            }
        }
        drop(applied);
    });
    wait_for_application
}

fn finish_active_search(frontend: &Frontend, generation: u64) {
    let _ =
        frontend
            .active_search
            .compare_exchange(generation, 0, Ordering::AcqRel, Ordering::Acquire);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_frontend(generation: u64, query: &str, passage_search_pending: bool) -> Frontend {
        let (generation_updates, _) = tokio::sync::watch::channel(generation);
        Frontend {
            generation: std::sync::atomic::AtomicU64::new(generation),
            active_search: std::sync::atomic::AtomicU64::new(0),
            generation_updates,
            interactive_search: tokio::sync::Mutex::new(()),
            model: std::sync::Mutex::new(super::super::FrontendModel {
                query: query.to_string(),
                passage_search_pending,
                accepted: Vec::new(),
                selected_file: None,
                catalog_ticks_until_refresh: super::super::CATALOG_REFRESH_TICKS,
                accepted_passages: Vec::new(),
                accepted_paths: Vec::new(),
                passages_loaded: false,
                displayed: Vec::new(),
                result_filter: "all".to_string(),
                content_view_passages: Vec::new(),
            }),
        }
    }

    #[test]
    fn pending_passage_timer_takes_only_the_latest_generation_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let frontend = test_frontend(1, "first", true);
        let cancelled_generation = frontend.generation_updates.subscribe();
        let stale_generation = frontend.generation.load(Ordering::Acquire);
        {
            let mut model = lock(&frontend.model);
            model.query = "latest".to_string();
            model.passage_search_pending = true;
        }

        let latest_generation = advance_search_generation(&frontend)
            .ok_or("search generation unexpectedly exhausted")?;
        assert!(cancelled_generation.has_changed()?);
        assert_eq!(*cancelled_generation.borrow(), latest_generation);
        assert_ne!(latest_generation, stale_generation);
        assert_eq!(
            take_pending_passage_search(&frontend),
            Some(("latest".to_string(), latest_generation))
        );
        assert_eq!(take_pending_passage_search(&frontend), None);
        Ok(())
    }

    #[tokio::test]
    async fn superseded_search_drops_while_waiting_for_the_shared_slot() {
        let frontend = test_frontend(1, "stale", false);
        let _held_slot = frontend.interactive_search.lock().await;
        let mut generation_updates = frontend.generation_updates.subscribe();
        let pending = acquire_interactive_search_slot(&frontend, 1, &mut generation_updates);
        tokio::pin!(pending);
        tokio::task::yield_now().await;

        assert_eq!(advance_search_generation(&frontend), Some(2));
        let acquired = tokio::time::timeout(std::time::Duration::from_millis(100), &mut pending)
            .await
            .expect("generation cancellation must release a queued search");
        assert!(acquired.is_none());
    }

    #[tokio::test]
    async fn superseded_catalog_gate_drops_passage_result() {
        let frontend = test_frontend(1, "stale", false);
        let mut generation_updates = frontend.generation_updates.subscribe();
        let (_catalog_applied, wait_for_catalog) = tokio::sync::oneshot::channel();
        let waiting =
            wait_for_catalog_application(&frontend, 1, &mut generation_updates, wait_for_catalog);
        tokio::pin!(waiting);

        assert_eq!(advance_search_generation(&frontend), Some(2));
        assert!(!waiting.await);
    }
}
