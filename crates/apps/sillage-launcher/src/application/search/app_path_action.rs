use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::super::window::show_notice;
use super::super::{Frontend, UiWeak, lock};
use super::passage_view::path_result_is_visible;
use crate::errors::LauncherError;
use crate::ipc::LauncherState;

pub(in crate::application) fn activate_path_action(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    result_id: String,
    action_id: String,
) {
    if action_id != "path.copy" || !path_result_is_visible(&frontend, &result_id) {
        show_notice(&ui, "This file path is no longer visible.".to_string());
        return;
    }
    let generation = frontend.generation.load(Ordering::Acquire);
    let selected = {
        let model = lock(&frontend.model);
        model
            .accepted_paths
            .iter()
            .find(|accepted| accepted.result_id == result_id)
            .map(|accepted| (model.query.clone(), accepted.path.clone()))
    };
    let Some((query, path)) = selected else {
        show_notice(
            &ui,
            "This file path belongs to an older search.".to_string(),
        );
        return;
    };
    if let Err(error) = state.begin_passage_action(generation) {
        show_notice(&ui, error.message);
        return;
    }
    if state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service())
        .is_none()
    {
        state.finish_action();
        show_notice(&ui, "Document search is not configured.".to_string());
        return;
    }

    runtime.spawn(async move {
        let fresh = super::search_passages(&frontend, &state, generation, &query, &ui)
            .await
            .is_some_and(|result| result.paths.iter().any(|entry| entry.path == path));
        let failed_delivery_state = Arc::clone(&state);
        if slint::invoke_from_event_loop(move || {
            let still_selected = {
                let model = lock(&frontend.model);
                model.query == query
                    && model
                        .accepted_paths
                        .iter()
                        .any(|entry| entry.result_id == result_id && entry.path == path)
            };
            if frontend.generation.load(Ordering::Acquire) != generation
                || state.ensure_generation(generation).is_err()
                || !still_selected
                || !path_result_is_visible(&frontend, &result_id)
            {
                state.finish_action();
                show_notice(
                    &ui,
                    "This file path belongs to an older search.".to_string(),
                );
                return;
            }
            let outcome = if fresh {
                super::super::platform::copy_text(&path)
                    .map(|()| "Copied freshly verified file path.".to_string())
            } else {
                Err(LauncherError::file_unavailable(
                    "This file path changed or is no longer authorized.",
                ))
            };
            state.finish_action();
            match outcome {
                Ok(message) => show_notice(&ui, message),
                Err(error) => show_notice(&ui, error.message),
            }
        })
        .is_err()
        {
            failed_delivery_state.finish_action();
        }
    });
}
