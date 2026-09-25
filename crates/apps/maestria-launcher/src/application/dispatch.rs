use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::platform::{choose_file, copy_text, validate_selected_path};
use super::preferences::sync_preferences;
use super::search::file_actions;
use super::window::{hide_launcher, show_notice};
use super::{FileSelection, Frontend, LauncherWindow, UiWeak, empty_results, lock};
use crate::ResultRow;
use crate::errors::LauncherError;
use crate::ipc::{ActionTarget, LauncherState};
use crate::model::ActionOutcome;
use crate::shortcuts::Shortcuts;

pub(super) struct DispatchContext {
    pub(super) state: Arc<LauncherState>,
    pub(super) frontend: Arc<Frontend>,
    pub(super) shortcuts: Arc<Shortcuts>,
    pub(super) runtime: tokio::runtime::Handle,
    pub(super) ui: UiWeak,
    pub(super) system_dark: bool,
}

pub(super) fn dispatch_action(result_id: String, action_id: String, context: DispatchContext) {
    let DispatchContext {
        state,
        frontend,
        shortcuts,
        runtime,
        ui,
        system_dark,
    } = context;
    let generation = frontend
        .generation
        .load(std::sync::atomic::Ordering::Acquire);
    let target = match state.begin_action(&result_id, &action_id, generation) {
        Ok(target) => target,
        Err(error) => {
            show_notice(&ui, error.message);
            return;
        }
    };

    match target {
        ActionTarget::OpenFile => dispatch_file_open(state, frontend, runtime, ui, generation),
        ActionTarget::ResetPreferences => {
            dispatch_reset(state, shortcuts, runtime, ui, system_dark);
        }
        target => {
            dispatch_immediate_action(target, &state, &shortcuts, generation, &ui, system_dark)
        }
    }
}

fn dispatch_file_open(
    state: Arc<LauncherState>,
    frontend: Arc<Frontend>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    generation: u64,
) {
    let Some(window) = ui.upgrade() else {
        state.finish_action();
        return;
    };
    if let Err(error) = state.begin_modal() {
        state.finish_action();
        show_notice(&ui, error.message);
        return;
    }
    #[cfg(target_os = "linux")]
    let parent = crate::platform::window_identifier(window.window(), &runtime);
    #[cfg(not(target_os = "linux"))]
    let parent = None;

    runtime.spawn(async move {
        let selection = choose_file(parent).await;
        state.end_modal();
        match selection {
            Ok(Some(path)) => {
                let result = validate_selected_path(&path)
                    .and_then(|()| state.install_selected_file(path, generation));
                state.finish_action();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = ui.upgrade() {
                        match result {
                            Ok((result_id, display_path)) => {
                                show_selected_file(&window, &frontend, result_id, display_path);
                            }
                            Err(error) => window.set_notice(error.message.into()),
                        }
                    }
                });
            }
            Ok(None) => state.finish_action(),
            Err(error) => {
                state.finish_action();
                show_notice(&ui, error.message);
            }
        }
    });
}

fn show_selected_file(
    window: &LauncherWindow,
    frontend: &Frontend,
    result_id: String,
    display_path: String,
) {
    {
        let mut model = lock(&frontend.model);
        model.selected_file = Some(FileSelection {
            result_id: result_id.clone(),
        });
        model.displayed.clear();
        model.content_view_passages.clear();
        model.result_filter = "all".to_string();
    }
    let row = ResultRow {
        id: result_id.into(),
        title: "Selected file".into(),
        subtitle: display_path.clone().into(),
        kind: "command".into(),
        accessible_name: format!("Selected file {display_path}").into(),
        excerpt_before: "".into(),
        excerpt_match: "".into(),
        excerpt_after: "".into(),
        content: "".into(),
    };
    window.set_passage_view_open(false);
    window.set_passage_view_results(empty_results());
    window.set_result_filter("all".into());
    window.set_results(ModelRc::new(VecModel::from(vec![row])));
    window.set_actions(file_actions());
    window.set_selected_index(0);
    window.set_selected_action_index(0);
    window.set_actions_open(true);
    window.set_status_kind("ready".into());
    window.set_status_message("Choose Open or Copy Path.".into());
}

fn dispatch_reset(
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    system_dark: bool,
) {
    let reset_state = Arc::clone(&state);
    let reset_shortcuts = Arc::clone(&shortcuts);
    runtime.spawn(async move {
        let confirmed = reset_state.settings().and_then(|mut settings| {
            settings
                .confirm_reset()
                .map_err(LauncherError::settings_failed)
        });
        let reset_result = match confirmed {
            Err(error) => Err(error),
            Ok(()) => match reset_shortcuts.clear().await {
                Err(error) => Err(error),
                Ok(_) => reset_state.settings().and_then(|mut settings| {
                    settings.reset().map_err(LauncherError::settings_failed)
                }),
            },
        };
        reset_state.finish_action();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = ui.upgrade() {
                window.set_reset_confirmation(false);
                match reset_result {
                    Ok(()) => {
                        let _ =
                            sync_preferences(&window, &reset_state, &reset_shortcuts, system_dark);
                        window.set_notice("Preferences reset to defaults.".into());
                    }
                    Err(error) => window.set_notice(error.message.into()),
                }
            }
        });
    });
}

fn dispatch_immediate_action(
    target: ActionTarget,
    state: &LauncherState,
    shortcuts: &Shortcuts,
    generation: u64,
    ui: &UiWeak,
    system_dark: bool,
) {
    let result = perform_action(target, state, shortcuts, generation);
    state.finish_action();
    match result {
        Ok(ActionOutcome::ShowPreferences) => {
            if let Some(window) = ui.upgrade() {
                window.set_preferences_open(true);
                if let Err(error) = sync_preferences(&window, state, shortcuts, system_dark) {
                    window.set_preferences_warning(error.message.into());
                }
                window.invoke_focus_preferences();
            }
        }
        Ok(ActionOutcome::Refreshed) => {
            if let Some(window) = ui.upgrade() {
                window.set_notice("Refreshing installed applications…".into());
            }
        }
        Ok(ActionOutcome::Copied { .. }) => {
            if let Some(window) = ui.upgrade() {
                window.set_notice("Copied to clipboard.".into());
            }
        }
        Ok(ActionOutcome::Dismiss) => hide_launcher(ui, state),
        Ok(ActionOutcome::FileSelected { .. } | ActionOutcome::Cancelled) => {}
        Err(error) => show_notice(ui, error.message),
    }
}

fn perform_action(
    target: ActionTarget,
    state: &LauncherState,
    shortcuts: &Shortcuts,
    generation: u64,
) -> Result<ActionOutcome, LauncherError> {
    match target {
        ActionTarget::OpenFile | ActionTarget::ResetPreferences => Err(
            LauncherError::invalid_request("Action requires asynchronous dispatch"),
        ),
        ActionTarget::ShowPreferences => {
            state.enter_preferences()?;
            Ok(ActionOutcome::ShowPreferences)
        }
        ActionTarget::Refresh => {
            state.catalog().request_refresh()?;
            Ok(ActionOutcome::Refreshed)
        }
        ActionTarget::Quit => {
            state.allow_exit()?;
            shortcuts.shutdown()?;
            slint::quit_event_loop()
                .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
            Ok(ActionOutcome::Dismiss)
        }
        ActionTarget::CopyActivation => {
            copy_text("maestria-launcher --activate")?;
            Ok(ActionOutcome::Copied { result_id: None })
        }
        ActionTarget::OpenSelectedFile(path) => {
            state.with_current_file(generation, &path, || {
                validate_selected_path(&path)?;
                crate::platform::open_local_file(&path)
            })?;
            state.dismiss()?;
            Ok(ActionOutcome::Dismiss)
        }
        ActionTarget::CopySelectedPath { path, result_id } => {
            state.with_current_file(generation, &path, || {
                let value = path.to_str().ok_or_else(|| {
                    LauncherError::file_unavailable("The selected file path is not valid Unicode")
                })?;
                copy_text(value)
            })?;
            Ok(ActionOutcome::Copied {
                result_id: Some(result_id),
            })
        }
        ActionTarget::CopyValue(value) => {
            copy_text(&value)?;
            Ok(ActionOutcome::Copied { result_id: None })
        }
        ActionTarget::OpenApplication(desktop_id) => {
            state.ensure_generation(generation)?;
            crate::platform::launch_app(&desktop_id)?;
            state.dismiss()?;
            Ok(ActionOutcome::Dismiss)
        }
    }
}
