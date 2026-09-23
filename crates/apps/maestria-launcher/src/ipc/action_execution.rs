use std::path::PathBuf;

use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::oneshot;

use crate::errors::LauncherError;
use crate::model::ActionOutcome;
use crate::shortcuts;

use super::LauncherState;

#[derive(Debug, Clone)]
pub enum ActionTarget {
    OpenFile,
    ShowPreferences,
    Refresh,
    Quit,
    CopyActivation,
    ResetPreferences,
    OpenSelectedFile(PathBuf),
    CopySelectedPath { path: PathBuf, result_id: String },
    OpenApplication(String),
    CopyValue(String),
}

pub(super) async fn execute_target(
    app: &AppHandle,
    window: &WebviewWindow,
    state: &State<'_, LauncherState>,
    target: ActionTarget,
    generation: u64,
) -> Result<ActionOutcome, LauncherError> {
    match target {
        ActionTarget::OpenFile => select_file(app, window, state, generation).await,
        ActionTarget::ShowPreferences => show_preferences(state),
        ActionTarget::Refresh => refresh_catalog(app, state),
        ActionTarget::Quit => quit_launcher(app, state),
        ActionTarget::CopyActivation => copy_text(app, "maestria-launcher --activate", None),
        ActionTarget::ResetPreferences => reset_preferences(app, state),
        ActionTarget::OpenSelectedFile(path) => {
            open_selected_file(app, window, state, path, generation)
        }
        ActionTarget::CopySelectedPath { path, result_id } => {
            state.with_current_file(generation, &path, || {
                copy_text(app, path.to_string_lossy().as_ref(), Some(result_id))
            })
        }
        ActionTarget::CopyValue(value) => copy_text(app, &value, None),
        ActionTarget::OpenApplication(desktop_id) => {
            open_application(app, window, desktop_id, generation).await
        }
    }
}

async fn select_file(
    app: &AppHandle,
    window: &WebviewWindow,
    state: &State<'_, LauncherState>,
    generation: u64,
) -> Result<ActionOutcome, LauncherError> {
    state.begin_modal()?;
    let _modal = super::ModalGuard::new(state);
    let (selected, selection) = oneshot::channel();
    app.dialog()
        .file()
        .set_parent(window)
        .set_title("Open a local file")
        .pick_file(move |file| {
            let _ = selected.send(file);
        });
    let picked = selection.await;
    let picked = picked
        .map_err(|_| LauncherError::file_unavailable("The file chooser closed unexpectedly"))?;
    let Some(file_path) = picked else {
        return Ok(ActionOutcome::Cancelled);
    };
    let path = file_path
        .into_path()
        .map_err(|error| LauncherError::file_unavailable(error.to_string()))?;
    validate_selected_path(&path)?;
    let (result_id, display_path) = state.install_selected_file(path, generation)?;
    Ok(ActionOutcome::FileSelected {
        result_id,
        display_path,
    })
}

fn validate_selected_path(path: &std::path::Path) -> Result<(), LauncherError> {
    if !path.is_absolute() {
        return Err(LauncherError::file_unavailable(
            "Only local filesystem paths are supported",
        ));
    }
    if path.to_str().is_none() {
        return Err(LauncherError::file_unavailable(
            "The selected file path is not valid Unicode",
        ));
    }
    if !path.is_file() {
        return Err(LauncherError::file_unavailable(
            "The selected file is unavailable",
        ));
    }
    Ok(())
}

fn show_preferences(state: &State<'_, LauncherState>) -> Result<ActionOutcome, LauncherError> {
    state.enter_preferences()?;
    Ok(ActionOutcome::ShowPreferences)
}

fn refresh_catalog(
    app: &AppHandle,
    state: &State<'_, LauncherState>,
) -> Result<ActionOutcome, LauncherError> {
    state.catalog().request_refresh(app)?;
    Ok(ActionOutcome::Refreshed)
}

fn quit_launcher(
    app: &AppHandle,
    state: &State<'_, LauncherState>,
) -> Result<ActionOutcome, LauncherError> {
    state.allow_exit()?;
    app.exit(0);
    Ok(ActionOutcome::Dismiss)
}

fn copy_text(
    app: &AppHandle,
    value: &str,
    result_id: Option<String>,
) -> Result<ActionOutcome, LauncherError> {
    app.clipboard()
        .write_text(value.to_string())
        .map(|()| ActionOutcome::Copied { result_id })
        .map_err(|error| LauncherError::clipboard_failed(error.to_string()))
}

fn reset_preferences(
    app: &AppHandle,
    state: &State<'_, LauncherState>,
) -> Result<ActionOutcome, LauncherError> {
    let mut settings = state.settings()?;
    settings
        .ensure_reset_confirmed()
        .map_err(LauncherError::settings_failed)?;
    shortcuts::clear(app)?;
    settings.reset().map_err(LauncherError::settings_failed)?;
    Ok(ActionOutcome::Refreshed)
}

fn open_selected_file(
    app: &AppHandle,
    window: &WebviewWindow,
    state: &State<'_, LauncherState>,
    path: PathBuf,
    generation: u64,
) -> Result<ActionOutcome, LauncherError> {
    state.with_current_file(generation, &path, || {
        validate_selected_path(&path)?;
        std::fs::File::open(&path).map_err(|error| {
            LauncherError::file_unavailable(format!("The selected file is not readable: {error}"))
        })?;
        app.opener()
            .open_path(path.to_string_lossy().into_owned(), None::<String>)
            .map_err(|error| LauncherError::launch_failed(error.to_string()))
    })?;
    if state.dismiss_generation(generation)? {
        window
            .hide()
            .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    }
    Ok(ActionOutcome::Dismiss)
}

async fn open_application(
    app: &AppHandle,
    window: &WebviewWindow,
    desktop_id: String,
    generation: u64,
) -> Result<ActionOutcome, LauncherError> {
    let window = window.clone();
    super::on_main_thread(app, move || {
        let state = window.state::<LauncherState>();
        state.ensure_generation(generation)?;
        crate::platform::launch_app(&window, &desktop_id)?;
        if state.dismiss_generation(generation)? {
            window
                .hide()
                .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
        }
        Ok(ActionOutcome::Dismiss)
    })
    .await
}
