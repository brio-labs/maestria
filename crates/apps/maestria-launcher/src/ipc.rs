mod action_execution;
mod query_worker;
mod session;

pub use action_execution::ActionTarget;
pub use query_worker::QueryWorker;
pub use session::LauncherState;

use std::sync::Arc;

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::errors::LauncherError;
use crate::metrics::{FramePhase, LauncherMetrics};
use crate::model::{
    ActionOutcome, LAUNCHER_WINDOW_LABEL, PreferencesDto, PreferencesUpdate, SearchResponse,
    ShortcutControl, ShortcutSetup, ShortcutState, ShortcutStatus,
};
use crate::shortcuts::{self, Shortcuts};

struct ModalGuard<'a> {
    state: &'a LauncherState,
}

impl<'a> ModalGuard<'a> {
    fn new(state: &'a LauncherState) -> Self {
        Self { state }
    }
}

impl Drop for ModalGuard<'_> {
    fn drop(&mut self) {
        self.state.end_modal();
    }
}

struct ActionFlight<'a> {
    state: &'a LauncherState,
}

impl<'a> ActionFlight<'a> {
    fn new(state: &'a LauncherState) -> Self {
        Self { state }
    }
}

impl Drop for ActionFlight<'_> {
    fn drop(&mut self) {
        self.state.finish_action();
    }
}

fn ensure_launcher_window(window: &WebviewWindow) -> Result<(), LauncherError> {
    if window.label() == LAUNCHER_WINDOW_LABEL {
        Ok(())
    } else {
        Err(LauncherError::invalid_request(
            "This command is restricted to the launcher window",
        ))
    }
}

/// Notify the native shell that renderer listeners and the initial UI are ready.
///
/// # Cancellation
///
/// Readiness is committed before presentation is queued. Dropping this future can
/// therefore still allow the queued show/focus effect to run; an already-started
/// shortcut restoration is detached and is not rolled back.
#[tauri::command]
pub async fn launcher_ready(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, LauncherState>,
) -> Result<(), LauncherError> {
    ensure_launcher_window(&window)?;
    let generation = state.mark_ready()?;
    app.state::<LauncherMetrics>()
        .initial_frame_requested(generation);
    let handle = app.clone();
    on_main_thread(&app, move || {
        crate::application::present_generation(&handle, generation)
    })
    .await?;
    let restore = {
        let settings = state.settings()?;
        (settings.shortcut_setup() == ShortcutSetup::Requested)
            .then(|| settings.shortcut().to_string())
    };
    if let Some(preferred) = restore {
        tauri::async_runtime::spawn(async move {
            if let Err(error) = configure_and_persist(app, preferred, false).await {
                eprintln!("Shortcut restoration failed: {}", error.message);
            }
        });
    }
    Ok(())
}

/// Search the accepted catalog for a generation-tagged renderer query.
///
/// # Cancellation
///
/// The generation and accepted-result set are cleared before worker submission.
/// Dropping this future does not stop the bounded worker, but a later generation
/// prevents its response from being accepted or decorated; no native launch or
/// clipboard effect is performed by this command.
#[tauri::command]
pub async fn search(
    window: WebviewWindow,
    state: State<'_, LauncherState>,
    query: String,
    generation: u64,
) -> Result<SearchResponse, LauncherError> {
    ensure_launcher_window(&window)?;
    window
        .app_handle()
        .state::<LauncherMetrics>()
        .results_requested(generation);
    let mut response = state.search(query, generation).await?;
    let apps = Arc::clone(&state.catalog().snapshot()?.apps);
    let app = window.app_handle().clone();
    on_main_thread(&app, move || {
        window
            .state::<LauncherState>()
            .ensure_generation(generation)?;
        crate::platform::decorate_results(&window, &apps, &mut response.results);
        Ok(response)
    })
    .await
}

/// Execute one action previously accepted for the current result generation.
///
/// # Cancellation
///
/// Generation and action-in-flight guards are acquired before any effect. Dropping
/// this future releases the action guard, but cannot undo a native effect already
/// dispatched (for example clipboard writes or an app launch); generation checks
/// still prevent stale application launches and stale file selections.
#[tauri::command]
pub async fn execute_action(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, LauncherState>,
    result_id: String,
    action_id: String,
    generation: u64,
) -> Result<ActionOutcome, LauncherError> {
    ensure_launcher_window(&window)?;
    let target = state.begin_action(&result_id, &action_id, generation)?;
    let _action = ActionFlight::new(&state);
    crate::ipc::action_execution::execute_target(&app, &window, &state, target, generation).await
}

/// Hide the launcher and discard the current accepted result and file selection.
#[tauri::command]
pub fn dismiss(
    window: WebviewWindow,
    state: State<'_, LauncherState>,
) -> Result<(), LauncherError> {
    ensure_launcher_window(&window)?;
    state.dismiss()?;
    window
        .hide()
        .map_err(|error| LauncherError::platform_unavailable(error.to_string()))
}

/// Enter Preferences and return the native settings plus binding status.
#[tauri::command]
pub fn get_preferences(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, LauncherState>,
) -> Result<PreferencesDto, LauncherError> {
    ensure_launcher_window(&window)?;
    state.enter_preferences()?;
    Ok(state.settings()?.dto(app.state::<Shortcuts>().status()))
}

/// Apply validated Preferences updates and, when needed, configure the native shortcut.
///
/// # Cancellation
///
/// Session settings are changed only after any requested native configuration returns.
/// Dropping the future during configuration does not roll back a binding already
/// accepted by the native adapter, but its modal guard is released and persistence
/// is not reported as complete until this command reaches its final update.
#[tauri::command]
pub async fn save_preferences(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, LauncherState>,
    mut request: PreferencesUpdate,
) -> Result<PreferencesDto, LauncherError> {
    ensure_launcher_window(&window)?;
    let first_deferral = request.shortcut_setup == Some(ShortcutSetup::Deferred)
        && request.shortcut.is_none()
        && request.reduce_motion.is_none()
        && request.confirm_reset.is_none()
        && state.settings()?.shortcut_setup() == ShortcutSetup::Unconfigured;
    if !state.preferences_active()? && !first_deferral {
        return Err(LauncherError::invalid_request(
            "Open Preferences before changing launcher settings",
        ));
    }
    if request.confirm_reset == Some(true) {
        let mut settings = state.settings()?;
        settings
            .confirm_reset()
            .map_err(LauncherError::settings_failed)?;
        return Ok(settings.dto(app.state::<Shortcuts>().status()));
    }
    if request
        .shortcut_setup
        .as_ref()
        .is_some_and(|setup| *setup != ShortcutSetup::Deferred)
    {
        return Err(LauncherError::invalid_request(
            "Registration intent is owned by shortcut setup",
        ));
    }
    let changed_shortcut = {
        let settings = state.settings()?;
        settings
            .validate_update(&request)
            .map_err(LauncherError::settings_failed)?;
        request
            .shortcut
            .as_ref()
            .filter(|shortcut| shortcut.as_str() != settings.shortcut())
            .cloned()
    };
    if let Some(preferred) = changed_shortcut {
        if app.state::<Shortcuts>().status().control != ShortcutControl::Application {
            return Err(LauncherError::invalid_request(
                "This shortcut is configured by the system",
            ));
        }
        let outcome = configure_native(&app, preferred, true).await?;
        request.shortcut_setup = Some(
            if outcome.denied && outcome.status.state != ShortcutState::Available {
                ShortcutSetup::Deferred
            } else {
                ShortcutSetup::Requested
            },
        );
    }
    let mut settings = state.settings()?;
    // Persistence errors are retained in the DTO warning; session changes still apply.
    settings
        .apply_update(&request)
        .map_err(LauncherError::settings_failed)?;
    Ok(settings.dto(app.state::<Shortcuts>().status()))
}

/// Request or rebind the native shortcut using the currently selected preference.
///
/// # Cancellation
///
/// The portal or global-shortcut adapter may have accepted a binding before the
/// future is dropped; that native effect is not rolled back. The modal guard is
/// released, while settings are persisted only after the adapter returns normally.
#[tauri::command]
pub async fn configure_shortcut(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, LauncherState>,
) -> Result<ShortcutStatus, LauncherError> {
    ensure_launcher_window(&window)?;
    let preferred = {
        let settings = state.settings()?;
        if !state.preferences_active()? && settings.shortcut_setup() != ShortcutSetup::Unconfigured
        {
            return Err(LauncherError::invalid_request(
                "Open Preferences before configuring a shortcut",
            ));
        }
        settings
            .validate_update(&PreferencesUpdate {
                reduce_motion: None,
                shortcut: None,
                shortcut_setup: None,
                confirm_reset: None,
            })
            .map_err(LauncherError::settings_failed)?;
        settings.shortcut().to_string()
    };
    configure_and_persist(app, preferred, true).await
}
/// Acknowledge an opt-in post-commit renderer frame, without exposing query content.
///
/// Rejected when instrumentation is disabled, the originating window differs,
/// or the generation is no longer awaiting a frame. This is renderer-ready,
/// not proof that the compositor has presented pixels.
#[tauri::command]
pub fn frame_ready(
    window: WebviewWindow,
    generation: u64,
    phase: FramePhase,
    renderer_elapsed_ms: f64,
) -> Result<(), LauncherError> {
    ensure_launcher_window(&window)?;
    window.app_handle().state::<LauncherMetrics>().frame_ready(
        generation,
        phase,
        renderer_elapsed_ms,
    )
}

async fn configure_native(
    app: &AppHandle,
    preferred: String,
    explicit: bool,
) -> Result<shortcuts::ConfigureOutcome, LauncherError> {
    let state = app.state::<LauncherState>();
    state.begin_modal()?;
    let _modal = ModalGuard::new(&state);
    shortcuts::configure(app.clone(), preferred, explicit).await
}

async fn configure_and_persist(
    app: AppHandle,
    preferred: String,
    explicit: bool,
) -> Result<ShortcutStatus, LauncherError> {
    let outcome = configure_native(&app, preferred.clone(), explicit).await?;
    let state = app.state::<LauncherState>();
    let mut settings = state.settings()?;
    let update = PreferencesUpdate {
        shortcut: Some(preferred),
        shortcut_setup: Some(
            if outcome.denied && outcome.status.state != ShortcutState::Available {
                ShortcutSetup::Deferred
            } else {
                ShortcutSetup::Requested
            },
        ),
        reduce_motion: None,
        confirm_reset: None,
    };
    settings
        .validate_update(&update)
        .map_err(LauncherError::settings_failed)?;
    // Keep native binding truth even when the preferences file cannot be replaced.
    settings
        .apply_update(&update)
        .map_err(LauncherError::settings_failed)?;
    Ok(outcome.status)
}

async fn on_main_thread<T: Send + 'static>(
    app: &AppHandle,
    effect: impl FnOnce() -> Result<T, LauncherError> + Send + 'static,
) -> Result<T, LauncherError> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = send.send(effect());
    })
    .map_err(|error| LauncherError::platform_unavailable(error.to_string()))?;
    receive
        .await
        .map_err(|_| LauncherError::platform_unavailable("The native operation was interrupted"))?
}
