use std::sync::Arc;

use tauri::AppHandle;

use super::ConfigureOutcome;
use super::state::ShortcutInner;
use super::status::{
    available_status, lock_state, set_status, unconfigured_status, unsupported_status,
};
use crate::errors::LauncherError;
use crate::model::{ShortcutConfigureAction, ShortcutControl};

pub(super) fn install_plugin(
    app: &AppHandle,
    inner: &Arc<ShortcutInner>,
) -> Result<bool, LauncherError> {
    let handler = move |app: &AppHandle,
                        _shortcut: &tauri_plugin_global_shortcut::Shortcut,
                        event: tauri_plugin_global_shortcut::ShortcutEvent| {
        if event.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
            return;
        }
        let _ = crate::request_activation(app);
    };
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(handler)
        .build();
    if let Err(error) = app.plugin(plugin) {
        let message = format!("X11 global shortcut plugin could not start: {error}");
        set_status(inner, unsupported_status(&message));
        return Ok(false);
    }
    lock_state(inner).plugin_installed = true;
    Ok(true)
}

pub(super) async fn configure(
    app: &AppHandle,
    inner: &Arc<ShortcutInner>,
    preferred: String,
) -> Result<ConfigureOutcome, LauncherError> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let current = lock_state(inner).x11_shortcut.clone();
    if current.as_deref() == Some(preferred.as_str()) {
        let status = available_status(ShortcutControl::Application, &preferred, None);
        set_status(inner, status.clone());
        return Ok(ConfigureOutcome {
            status,
            denied: false,
        });
    }

    // Register the replacement before releasing the old grab. This preserves a
    // working binding when another process owns the requested accelerator.
    app.global_shortcut()
        .register(preferred.as_str())
        .map_err(|error| {
            LauncherError::new(
                "shortcut_unavailable",
                format!("the requested X11 shortcut is unavailable: {error}"),
                true,
            )
        })?;

    if let Some(old) = current.as_deref()
        && let Err(error) = app.global_shortcut().unregister(old)
    {
        let _ = app.global_shortcut().unregister(preferred.as_str());
        return Err(LauncherError::new(
            "shortcut_unavailable",
            format!("the previous X11 shortcut could not be released: {error}"),
            true,
        ));
    }
    lock_state(inner).x11_shortcut = Some(preferred.clone());

    let status = available_status(ShortcutControl::Application, &preferred, None);
    set_status(inner, status.clone());
    Ok(ConfigureOutcome {
        status,
        denied: false,
    })
}

pub(super) fn clear(app: &AppHandle, inner: &Arc<ShortcutInner>) -> Result<(), LauncherError> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let current = lock_state(inner).x11_shortcut.clone();
    if let Some(current) = current {
        app.global_shortcut()
            .unregister(current.as_str())
            .map_err(|error| {
                LauncherError::new(
                    "shortcut_unavailable",
                    format!("the active X11 shortcut could not be released: {error}"),
                    true,
                )
            })?;
        lock_state(inner).x11_shortcut = None;
    }
    let status = unconfigured_status(
        ShortcutControl::Application,
        super::DEFAULT_SHORTCUT,
        None,
        ShortcutConfigureAction::Setup,
    );
    set_status(inner, status);
    Ok(())
}

pub(super) fn shutdown(app: &AppHandle, current: Option<String>) -> Result<(), LauncherError> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    if let Some(current) = current {
        app.global_shortcut()
            .unregister(current.as_str())
            .map_err(|error| {
                LauncherError::new(
                    "shortcut_unavailable",
                    format!("the active X11 shortcut could not be released: {error}"),
                    true,
                )
            })?;
    }
    Ok(())
}
