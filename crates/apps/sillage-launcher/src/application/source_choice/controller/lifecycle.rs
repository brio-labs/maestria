mod prepare;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use sillage_daemon::SessionPreparationOwner;
use sillage_domain::{ControlPersistenceChoice, DocumentaryRetentionChoice, SourceScope};
use tokio::sync::{mpsc, oneshot};

use super::super::search::clear_selected_source_results;
#[cfg(test)]
use super::super::state::LifecycleTestStage;
use super::super::state::SessionSourceState;
use super::retained_root;
use crate::application::search_setup::SearchSetupController;
use crate::application::{Frontend, UiWeak};
use crate::settings::SettingsManager;

use crate::ipc::LauncherState;
use prepare::prepare_source;
#[cfg(test)]
use prepare::settle_prepare_failure;

#[cfg(target_os = "linux")]
pub(super) type SourceParent = Option<ashpd::WindowIdentifier>;
#[cfg(not(target_os = "linux"))]
pub(super) type SourceParent = Option<()>;

pub(super) enum SourceRequest {
    Choose(SourceParent),
    Retire(Arc<AtomicBool>),
    Prepare,
    Purge,
    Remove,
    Shutdown(oneshot::Sender<Result<(), String>>),
}

pub(super) async fn source_worker(
    state: Arc<SessionSourceState>,
    launcher_state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    ui: UiWeak,
    folder_search_setup: Arc<Mutex<Option<Arc<SearchSetupController>>>>,
    mut requests: mpsc::Receiver<SourceRequest>,
) -> Result<(), String> {
    while let Some(request) = requests.recv().await {
        match request {
            SourceRequest::Choose(parent) => {
                choose_source(
                    &state,
                    &launcher_state,
                    &frontend,
                    &ui,
                    &folder_search_setup,
                    parent,
                )
                .await;
            }
            SourceRequest::Retire(enqueued) => {
                enqueued.store(false, Ordering::Release);
                retire_source(&state, &frontend, &ui).await;
            }
            SourceRequest::Prepare => prepare_source(&state).await,
            SourceRequest::Purge => purge_source(&state, &frontend, &ui).await,
            SourceRequest::Remove => {
                remove_source(
                    &state,
                    &launcher_state,
                    &frontend,
                    &ui,
                    &folder_search_setup,
                )
                .await;
            }
            SourceRequest::Shutdown(settled) => {
                state.stop_new_admission();
                clear_selected_source_results(
                    &frontend,
                    &ui,
                    "Selected-source results were cleared during shutdown".to_string(),
                );
                let result = state.retire_active().await;
                let _ = settled.send(result);
                return Ok(());
            }
        }
    }
    state.stop_new_admission();
    clear_selected_source_results(
        &frontend,
        &ui,
        "Selected-source results were cleared during shutdown".to_string(),
    );
    state.retire_active().await
}

async fn choose_source(
    state: &SessionSourceState,
    launcher_state: &LauncherState,
    frontend: &Frontend,
    ui: &UiWeak,
    folder_search_setup: &Mutex<Option<Arc<SearchSetupController>>>,
    parent: SourceParent,
) {
    let chooser = super::super::super::platform::choose_markdown_file(parent);
    let selection = tokio::select! {
        biased;
        _ = state.wait_until_stopping() => None,
        result = chooser => Some(result),
    };
    launcher_state.end_modal();
    state.finish_picker();
    if state.is_stopping() {
        return;
    }
    let Some(selection) = selection else {
        return;
    };
    match selection {
        Ok(Some(path)) => match super::super::super::platform::validate_markdown_file(&path) {
            Ok(path) => {
                state.set_retiring();
                let Some(generation) =
                    crate::application::search::advance_search_generation(frontend)
                else {
                    if state.selected_path().is_none() {
                        state.finish_retirement();
                    }
                    state
                        .set_error("Search generation exhausted; restart the launcher".to_string());
                    return;
                };
                frontend.active_search.store(0, Ordering::Release);
                state.query_changed(generation);
                clear_selected_source_results(
                    frontend,
                    ui,
                    "Previous selected-source results were withdrawn before replacement"
                        .to_string(),
                );
                let Some(search_setup) = attached_folder_search_setup(folder_search_setup) else {
                    if state.selected_path().is_none() {
                        state.finish_retirement();
                    }
                    state.set_error(
                        "Managed folder-search ownership is unavailable; source selection was not admitted"
                            .to_string(),
                    );
                    return;
                };
                if let Err(error) = search_setup.suspend_for_source_choice().await {
                    let error = restore_unselected_source_route(state, &search_setup, error).await;
                    state.set_error(error);
                    return;
                }
                if let Err(error) = state.retire_active().await {
                    let error = restore_unselected_source_route(state, &search_setup, error).await;
                    state.set_error(error);
                    return;
                }
                if let Err(error) = commit_selected_source_path(state, launcher_state, &path) {
                    let error = restore_unselected_source_route(state, &search_setup, error).await;
                    state.set_error(error);
                }
            }
            Err(error) => state.set_error(error.message),
        },
        Ok(None) => {}
        Err(error) => state.set_error(error.message),
    }
}

async fn retire_source(state: &SessionSourceState, frontend: &Frontend, ui: &UiWeak) {
    state.set_retiring();
    clear_selected_source_results(
        frontend,
        ui,
        "Selected-source results were withdrawn after read authorization was removed".to_string(),
    );
    if let Err(error) = state.retire_active().await {
        state.set_error(error);
        return;
    }
    state.finish_retirement();
}

async fn purge_source(state: &SessionSourceState, frontend: &Frontend, ui: &UiWeak) {
    let Some(path) = state.selected_path() else {
        state.set_error(
            "Choose the Markdown file whose retained data you want to purge".to_string(),
        );
        return;
    };
    state.set_retiring();
    clear_selected_source_results(
        frontend,
        ui,
        "Selected-source results were withdrawn before purge".to_string(),
    );
    if let Err(error) = state.retire_active().await {
        state.set_error(error);
        return;
    }
    let root = match retained_root(&path) {
        Ok(root) => root,
        Err(error) => {
            state.finish_retirement();
            state.set_error(error);
            return;
        }
    };
    let owner = match SessionPreparationOwner::with_retention_root(root) {
        Ok(owner) => owner,
        Err(error) => {
            state.finish_retirement();
            state.set_error(format!("{error:#}"));
            return;
        }
    };
    let purge_result = owner.purge_retained_source(&path);
    owner.wait_for_idle().await;
    if let Err(error) = purge_result {
        state.finish_retirement();
        state.set_error(format!("{error:#}"));
        return;
    }
    state.set_selection(path);
    clear_selected_source_results(
        frontend,
        ui,
        "Retained data for this selected Markdown file was purged; reading requires fresh consent and Prepare".to_string(),
    );
}

async fn remove_source(
    state: &SessionSourceState,
    launcher_state: &LauncherState,
    frontend: &Frontend,
    ui: &UiWeak,
    folder_search_setup: &Mutex<Option<Arc<SearchSetupController>>>,
) {
    if state.selected_path().is_none() {
        state.set_error("There is no selected Markdown source to remove".to_string());
        return;
    }
    state.set_retiring();
    clear_selected_source_results(
        frontend,
        ui,
        "Selected-source results were withdrawn before removal".to_string(),
    );
    if let Err(error) = state.retire_active().await {
        state.set_error(error);
        return;
    }
    let transition = state.control_metadata_transition();
    let snapshot = state.snapshot();

    #[cfg(test)]
    state.wait_at_lifecycle_test_gate(LifecycleTestStage::RemoveAfterSnapshot, true, None);
    let persisted = launcher_state
        .settings()
        .map_err(|error| error.message)
        .and_then(|mut settings| {
            settings.set_source_control_preferences(
                snapshot.documentary_retention,
                snapshot.control_persistence,
                None,
            )
        });
    if let Err(error) = persisted {
        state.finish_retirement();
        state.set_error(error);
        return;
    }
    #[cfg(test)]
    state.wait_at_lifecycle_test_gate(LifecycleTestStage::RemoveAfterSettingsWrite, true, None);
    state.clear_selection();
    drop(transition);
    let message = if snapshot.documentary_retention
        == DocumentaryRetentionChoice::RetainUntilExplicitlyPurged
    {
        "The selected source was removed. Its retained data remains until you explicitly purge it."
    } else {
        "The selected Markdown source was removed."
    };
    clear_selected_source_results(frontend, ui, message.to_string());
    if let Some(search_setup) = attached_folder_search_setup(folder_search_setup)
        && let Err(error) = search_setup.resume_after_source_removal().await
    {
        state.set_error(error);
    }
}

fn attached_folder_search_setup(
    setup: &Mutex<Option<Arc<SearchSetupController>>>,
) -> Option<Arc<SearchSetupController>> {
    crate::application::lock(setup).clone()
}

fn persist_selected_source_path(
    settings: &mut SettingsManager,
    documentary_retention: DocumentaryRetentionChoice,
    control_persistence: ControlPersistenceChoice,
    path: &Path,
) -> Result<(), String> {
    if control_persistence != ControlPersistenceChoice::PersistExplicitly {
        return Ok(());
    }
    let scope = SourceScope::file(path.to_path_buf()).map_err(|error| error.to_string())?;
    settings.set_source_control_preferences(documentary_retention, control_persistence, Some(scope))
}

pub(super) fn commit_selected_source_path(
    state: &SessionSourceState,
    launcher_state: &LauncherState,
    path: &Path,
) -> Result<(), String> {
    let _transition = state.control_metadata_transition();
    let snapshot = state.snapshot();
    let mut settings = launcher_state.settings().map_err(|error| error.message)?;
    persist_selected_source_path(
        &mut settings,
        snapshot.documentary_retention,
        snapshot.control_persistence,
        path,
    )?;
    drop(settings);
    state.set_selection(path.to_path_buf());
    Ok(())
}

pub(super) fn update_source_controls(
    state: &SessionSourceState,
    launcher_state: &LauncherState,
    documentary_retention: DocumentaryRetentionChoice,
    control_persistence: ControlPersistenceChoice,
) -> Result<(), String> {
    let _transition = state.control_metadata_transition();
    let previous = state.snapshot();
    state.set_controls(documentary_retention, control_persistence)?;
    let selected_scope = match control_persistence {
        ControlPersistenceChoice::PersistExplicitly => match previous.selected_path {
            Some(path) => SourceScope::file(path).ok(),
            None => None,
        },
        ControlPersistenceChoice::SessionOnly => None,
    };
    let persisted = match launcher_state.settings() {
        Ok(mut settings) => settings.set_source_control_preferences(
            documentary_retention,
            control_persistence,
            selected_scope,
        ),
        Err(error) => Err(error.message),
    };
    if let Err(error) = persisted {
        let _ = state.set_controls(previous.documentary_retention, previous.control_persistence);
        return Err(error);
    }
    Ok(())
}

async fn restore_unselected_source_route(
    state: &SessionSourceState,
    search_setup: &Arc<SearchSetupController>,
    failure: String,
) -> String {
    if state.selected_path().is_none() {
        state.finish_retirement();
        if let Err(restore) = search_setup.resume_after_source_removal().await {
            return format!("{failure}; folder-search restoration failed: {restore}");
        }
    }
    failure
}

#[cfg(test)]
mod tests;
