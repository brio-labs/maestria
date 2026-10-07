use std::sync::Arc;

use super::UiWeak;
use super::window::{shortcut_label, show_notice};
use crate::LauncherWindow;
use crate::errors::LauncherError;
use crate::ipc::LauncherState;
use crate::model::{PreferencesUpdate, ShortcutSetup, ShortcutState};
use crate::shortcuts::Shortcuts;
pub(super) fn configure_saved_shortcut(
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
) {
    let saved = state.settings().map(|settings| {
        (
            settings.shortcut().to_string(),
            settings.shortcut_setup() == ShortcutSetup::Requested,
        )
    });
    let Ok((preferred, requested)) = saved else {
        return;
    };
    if !requested {
        return;
    }
    runtime.spawn(async move {
        let status = shortcuts.configure(&preferred, false).await;
        match status {
            Ok(status) => {
                post_shortcut_status(
                    &ui,
                    status,
                    Arc::clone(&state),
                    Arc::clone(&shortcuts),
                    false,
                );
            }
            Err(error) => show_notice(&ui, error.message),
        }
    });
}
pub(super) fn configure_shortcut(
    preferred: String,
    explicit: bool,
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    system_dark: bool,
) {
    let read_only = state
        .settings()
        .map(|settings| settings.dto(shortcuts.status()).read_only);
    if !matches!(read_only, Ok(false)) {
        show_notice(
            &ui,
            "Preferences are read-only until the incompatible file is reset.".to_string(),
        );
        return;
    }
    runtime.spawn(async move {
        match shortcuts.configure(&preferred, explicit).await {
            Ok(status) => {
                let update_result = if status.state == ShortcutState::Available {
                    apply_settings(
                        &state,
                        PreferencesUpdate {
                            shortcut: Some(preferred),
                            shortcut_setup: Some(ShortcutSetup::Requested),
                            reduce_motion: None,
                            theme: None,
                            confirm_reset: None,
                        },
                    )
                } else {
                    Ok(())
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = ui.upgrade() {
                        window.set_shortcut_status(shortcut_label(&status).into());
                        match update_result {
                            Ok(()) => {
                                if let Err(error) =
                                    sync_preferences(&window, &state, &shortcuts, system_dark)
                                {
                                    window.set_preferences_warning(error.message.into());
                                } else if status.state == ShortcutState::Available
                                    && !window.get_preferences_open()
                                {
                                    window.invoke_focus_search();
                                }
                            }
                            Err(error) => window.set_notice(error.message.into()),
                        }
                    }
                });
            }
            Err(error) => show_notice(&ui, error.message),
        }
    });
}
pub(super) fn defer_shortcut_setup(
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    ui: UiWeak,
    system_dark: bool,
) {
    let update_result = apply_settings(
        &state,
        PreferencesUpdate {
            shortcut: None,
            shortcut_setup: Some(ShortcutSetup::Deferred),
            reduce_motion: None,
            theme: None,
            confirm_reset: None,
        },
    );
    let Some(window) = ui.upgrade() else {
        return;
    };
    match update_result {
        Ok(()) => {
            if let Err(error) = sync_preferences(&window, &state, &shortcuts, system_dark) {
                window.set_preferences_warning(error.message.into());
            } else {
                window.invoke_focus_search();
            }
        }
        Err(error) => window.set_notice(error.message.into()),
    }
}
pub(super) fn clear_shortcut(
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    system_dark: bool,
) {
    let read_only = state
        .settings()
        .map(|settings| settings.dto(shortcuts.status()).read_only);
    if !matches!(read_only, Ok(false)) {
        show_notice(
            &ui,
            "Preferences are read-only until the incompatible file is reset.".to_string(),
        );
        return;
    }
    runtime.spawn(async move {
        match shortcuts.clear().await {
            Ok(status) => {
                let update_result = apply_settings(
                    &state,
                    PreferencesUpdate {
                        shortcut: None,
                        shortcut_setup: Some(ShortcutSetup::Unconfigured),
                        reduce_motion: None,
                        theme: None,
                        confirm_reset: None,
                    },
                );
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = ui.upgrade() {
                        window.set_shortcut_status(shortcut_label(&status).into());
                        if let Err(error) = update_result {
                            window.set_notice(error.message.into());
                        } else if let Err(error) =
                            sync_preferences(&window, &state, &shortcuts, system_dark)
                        {
                            window.set_preferences_warning(error.message.into());
                        }
                    }
                });
            }
            Err(error) => show_notice(&ui, error.message),
        }
    });
}
pub(super) fn save_preferences(
    mut update: PreferencesUpdate,
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    runtime: tokio::runtime::Handle,
    ui: UiWeak,
    system_dark: bool,
) {
    let Some(shortcut) = update.shortcut.take() else {
        show_notice(&ui, "Preferences are read-only or unavailable.".to_string());
        return;
    };
    let current = state.settings().map(|settings| {
        (
            settings.shortcut().to_string(),
            settings.shortcut_setup(),
            settings.dto(shortcuts.status()).read_only,
        )
    });
    let Ok((current_shortcut, setup, false)) = current else {
        show_notice(&ui, "Preferences are read-only or unavailable.".to_string());
        return;
    };
    let reconfigure = current_shortcut != shortcut && setup == ShortcutSetup::Requested;
    runtime.spawn(async move {
        if reconfigure {
            match shortcuts.configure(&shortcut, true).await {
                Ok(status) if status.state == ShortcutState::Available => {}
                Ok(status) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = ui.upgrade() {
                            window.set_shortcut_status(shortcut_label(&status).into());
                            window.set_notice(
                                "The shortcut was not approved; other preferences were not saved."
                                    .into(),
                            );
                        }
                    });
                    return;
                }
                Err(error) => {
                    show_notice(&ui, error.message);
                    return;
                }
            }
        }
        update.shortcut = Some(shortcut);
        update.shortcut_setup = reconfigure.then_some(ShortcutSetup::Requested);
        let update_result = apply_settings(&state, update);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = ui.upgrade() {
                match update_result {
                    Ok(()) => {
                        if let Err(error) =
                            sync_preferences(&window, &state, &shortcuts, system_dark)
                        {
                            window.set_preferences_warning(error.message.into());
                        } else {
                            window.set_notice("Preferences saved.".into());
                            window.set_dark_mode(theme_is_dark(&state, system_dark));
                        }
                    }
                    Err(error) => window.set_preferences_warning(error.message.into()),
                }
            }
        });
    });
}
fn apply_settings(state: &LauncherState, update: PreferencesUpdate) -> Result<(), LauncherError> {
    let mut settings = state.settings()?;
    settings
        .validate_update(&update)
        .map_err(LauncherError::settings_failed)?;
    settings
        .apply_update(&update)
        .map_err(LauncherError::settings_failed)
}
pub(super) fn sync_preferences(
    ui: &LauncherWindow,
    state: &LauncherState,
    shortcuts: &Shortcuts,
    system_dark: bool,
) -> Result<(), LauncherError> {
    let preferences = state.settings()?.dto(shortcuts.status());
    ui.set_shortcut_offer_visible(
        !preferences.read_only && preferences.shortcut_setup == ShortcutSetup::Unconfigured,
    );
    ui.set_shortcut(preferences.shortcut.clone().into());
    ui.set_theme_choice(preferences.theme.clone().into());
    ui.set_reduce_motion_choice(preferences.reduce_motion);
    let warning = preferences
        .warning
        .as_deref()
        .map_or_else(String::new, str::to_owned);
    ui.set_preferences_warning(warning.into());
    ui.set_preferences_read_only(preferences.read_only);
    ui.set_system_dark_mode(system_dark);
    ui.set_dark_mode(match preferences.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => system_dark,
    });
    ui.set_shortcut_status(shortcut_label(&preferences.shortcut_status).into());
    Ok(())
}
fn theme_is_dark(state: &LauncherState, system_dark: bool) -> bool {
    state
        .settings()
        .map_or(system_dark, |settings| match settings.theme() {
            "dark" => true,
            "light" => false,
            _ => system_dark,
        })
}
fn post_shortcut_status(
    ui: &UiWeak,
    status: crate::model::ShortcutStatus,
    state: Arc<LauncherState>,
    shortcuts: Arc<Shortcuts>,
    system_dark: bool,
) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_shortcut_status(shortcut_label(&status).into());
            if let Err(error) = sync_preferences(&window, &state, &shortcuts, system_dark) {
                window.set_preferences_warning(error.message.into());
            }
        }
    });
}
