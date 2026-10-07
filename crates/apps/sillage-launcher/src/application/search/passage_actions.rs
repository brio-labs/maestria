use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ModelRc, VecModel};

use super::super::passages::ReopenError;
use super::super::window::show_notice;
use super::super::{AcceptedPassage, Frontend, UiWeak, lock};
use super::passage_view::{passage_result_is_visible, show_reopened_passage};
use crate::ActionRow;
use crate::errors::LauncherError;
use crate::ipc::LauncherState;
use crate::settings::SearchServiceConfig;

pub(super) fn passage_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(vec![
        ActionRow {
            id: "passage.open-source".into(),
            title: "Show Passage + Open Source".into(),
            accessible_name: "Reopen current evidence, show its cited passage, and open its source when available".into(),
        },
        ActionRow {
            id: "passage.copy-citation".into(),
            title: "Copy Citation".into(),
            accessible_name: "Reopen the evidence and copy its citation".into(),
        },
        ActionRow {
            id: "passage.copy-excerpt".into(),
            title: "Copy Passage".into(),
            accessible_name: "Reopen the evidence and copy its passage".into(),
        },
        ActionRow {
            id: "passage.reveal-source".into(),
            title: "Open Source Folder".into(),
            accessible_name: "Reopen the evidence and open its containing source folder".into(),
        },
        ActionRow {
            id: "passage.copy-path".into(),
            title: "Copy Source Path".into(),
            accessible_name: "Reopen the evidence and copy its source path".into(),
        },
    ]))
}

pub(super) fn path_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::from(vec![ActionRow {
        id: "path.copy".into(),
        title: "Copy Path".into(),
        accessible_name: "Copy the authorized matching file path".into(),
    }]))
}

pub(in crate::application) fn activate_passage_action(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    result_id: String,
    action_id: String,
) {
    let Some((generation, accepted, config)) =
        begin_passage_action(&state, &frontend, &ui, &result_id, &action_id)
    else {
        return;
    };

    runtime.spawn(async move {
        let prepared = reopen_passage_action(config, &accepted, &action_id).await;
        let failed_delivery_state = Arc::clone(&state);
        if slint::invoke_from_event_loop(move || {
            finish_passage_action(state, frontend, ui, generation, accepted, prepared);
        })
        .is_err()
        {
            failed_delivery_state.finish_action();
        }
    });
}

fn begin_passage_action(
    state: &LauncherState,
    frontend: &Frontend,
    ui: &UiWeak,
    result_id: &str,
    action_id: &str,
) -> Option<(u64, AcceptedPassage, SearchServiceConfig)> {
    if !matches!(
        action_id,
        "passage.open-source"
            | "passage.copy-citation"
            | "passage.copy-excerpt"
            | "passage.reveal-source"
            | "passage.copy-path"
    ) {
        show_notice(ui, "This passage action is not available.".to_string());
        return None;
    }
    if !passage_result_is_visible(frontend, result_id) {
        show_notice(
            ui,
            "This passage is no longer visible in the current search.".to_string(),
        );
        return None;
    }
    let generation = frontend.generation.load(Ordering::Acquire);
    let accepted = lock(&frontend.model)
        .accepted_passages
        .iter()
        .find(|accepted| accepted.result_id == result_id)
        .cloned();
    let Some(accepted) = accepted else {
        show_notice(ui, "This passage belongs to an older search.".to_string());
        return None;
    };
    if frontend.generation.load(Ordering::Acquire) != generation {
        show_notice(ui, "This passage belongs to an older search.".to_string());
        return None;
    }
    if let Err(error) = state.begin_passage_action(generation) {
        show_notice(ui, error.message);
        return None;
    }
    let config = state
        .settings()
        .ok()
        .and_then(|settings| settings.search_service());
    let Some(config) = config else {
        state.finish_action();
        show_notice(ui, "Document search is not configured.".to_string());
        return None;
    };
    Some((generation, accepted, config))
}

async fn reopen_passage_action(
    config: SearchServiceConfig,
    accepted: &AcceptedPassage,
    action_id: &str,
) -> Result<PassageAction, String> {
    super::super::passages::reopen(config, &accepted.passage)
        .await
        .map_err(action_error)
        .and_then(|reopened| match action_id {
            "passage.copy-citation" => Ok(PassageAction::Copy(
                reopened.citation,
                "Copied citation.".to_string(),
            )),
            "passage.copy-excerpt" => Ok(PassageAction::Copy(
                reopened.excerpt,
                "Copied reopened passage.".to_string(),
            )),
            "passage.copy-path" => reopened
                .path
                .map(|path| {
                    PassageAction::Copy(
                        path.display().to_string(),
                        "Copied freshly reopened source path.".to_string(),
                    )
                })
                .ok_or_else(|| "This passage has no local source path.".to_string()),
            "passage.reveal-source" => reopened
                .path
                .map(PassageAction::Reveal)
                .ok_or_else(|| "This passage has no local source folder.".to_string()),
            "passage.open-source" => Ok(PassageAction::Open {
                path: reopened.path,
                pdf_page: reopened.pdf_page,
                excerpt: reopened.excerpt,
                citation: reopened.citation,
                fallback: reopened.fallback,
            }),
            _ => Err("This passage action is not available.".to_string()),
        })
}

fn finish_passage_action(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    ui: UiWeak,
    generation: u64,
    accepted: AcceptedPassage,
    prepared: Result<PassageAction, String>,
) {
    let still_accepted = lock(&frontend.model)
        .accepted_passages
        .iter()
        .any(|current| {
            current.result_id == accepted.result_id
                && current.passage.evidence_id == accepted.passage.evidence_id
                && current.passage.artifact_version == accepted.passage.artifact_version
        });
    let current_generation = frontend.generation.load(Ordering::Acquire) == generation
        && state.ensure_generation(generation).is_ok()
        && still_accepted;
    if !current_generation {
        state.finish_action();
        show_notice(&ui, "This passage belongs to an older search.".to_string());
        return;
    }
    let outcome = match prepared {
        Ok(PassageAction::Copy(value, confirmation)) => {
            super::super::platform::copy_text(&value).map(|()| confirmation)
        }
        Ok(PassageAction::Reveal(path)) => crate::platform::open_containing_folder(&path)
            .map(|()| "Opened the freshly reopened source folder.".to_string()),
        Ok(PassageAction::Open {
            path,
            pdf_page,
            excerpt,
            citation,
            fallback,
        }) => path
            .as_deref()
            .map_or(Ok(()), super::super::platform::validate_selected_path)
            .and_then(|()| {
                if let Some(window) = ui.upgrade() {
                    show_reopened_passage(&window, &frontend, &accepted, &citation, &excerpt);
                }
                match path {
                    Some(path) => match pdf_page {
                        Some(page) => crate::platform::open_local_pdf_page(&path, page),
                        None => crate::platform::open_local_file(&path),
                    }
                    .map(|()| format!("Opened the source. {fallback}")),
                    None => Ok(
                        "No local source file is available; the reopened passage is shown here."
                            .to_string(),
                    ),
                }
            }),
        Err(message) => Err(LauncherError::file_unavailable(message)),
    };
    state.finish_action();
    match outcome {
        Ok(message) => {
            if let Some(window) = ui.upgrade() {
                window.set_notice(message.clone().into());
                window.set_status_kind("ready".into());
                window.set_status_message(message.into());
            }
        }
        Err(error) => show_notice(&ui, error.message),
    }
}

fn action_error(error: ReopenError) -> String {
    match error {
        ReopenError::Unavailable => {
            "The document could not be reopened; search service may be unavailable.".to_string()
        }
        ReopenError::Changed => {
            "The source changed or access was revoked. Search again before taking action."
                .to_string()
        }
    }
}

enum PassageAction {
    Copy(String, String),
    Reveal(std::path::PathBuf),
    Open {
        path: Option<std::path::PathBuf>,
        pdf_page: Option<u32>,
        excerpt: String,
        citation: String,
        fallback: String,
    },
}
