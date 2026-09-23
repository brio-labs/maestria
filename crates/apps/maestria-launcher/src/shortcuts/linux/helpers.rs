use tauri::{AppHandle, Manager};

use crate::errors::LauncherError;
use crate::model::LAUNCHER_WINDOW_LABEL;
use crate::platform;

pub(super) fn dispatch_activation(
    app: &AppHandle,
    activated: &ashpd::desktop::global_shortcuts::Activated,
) {
    let token = ["activation-token", "activation_token"]
        .iter()
        .find_map(|key| {
            activated
                .options()
                .get(*key)
                .and_then(|value| <&str>::try_from(value).ok())
                .map(str::to_owned)
        });
    let dispatcher = app.clone();
    let activation_app = app.clone();
    let _ = dispatcher.run_on_main_thread(move || {
        if let Some(window) = activation_app.get_webview_window(LAUNCHER_WINDOW_LABEL) {
            if let Some(token) = token.as_deref() {
                let _ = platform::apply_activation_token(&window, token);
            }
            let _ = crate::request_activation(&activation_app);
        }
    });
}

pub(super) fn validate_accelerator(value: &str) -> Result<(), LauncherError> {
    if value.trim().is_empty() || value.as_bytes().contains(&0) {
        return Err(LauncherError::invalid_request(
            "shortcut cannot be empty or contain NUL",
        ));
    }
    value
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map(|_| ())
        .map_err(|error| {
            LauncherError::invalid_request(format!("invalid shortcut accelerator: {error}"))
        })
}

pub(super) fn portal_trigger(value: &str) -> Result<String, LauncherError> {
    validate_accelerator(value)?;
    let mut parts = Vec::new();
    for token in value.split('+').map(str::trim) {
        let upper = token.to_ascii_uppercase();
        let canonical = match upper.as_str() {
            "CTRL" | "CONTROL" => "CTRL",
            "ALT" | "OPTION" => "ALT",
            "SHIFT" => "SHIFT",
            "SUPER" | "CMD" | "COMMAND" => "SUPER",
            _ => token,
        };
        parts.push(
            if canonical.len() <= 5 && canonical.chars().all(|ch| ch.is_ascii_uppercase()) {
                canonical.to_ascii_lowercase()
            } else {
                canonical.to_string()
            },
        );
    }
    if parts.is_empty() {
        return Err(LauncherError::invalid_request("shortcut has no trigger"));
    }
    let mut trigger = parts.join("+");
    if value.eq_ignore_ascii_case(super::DEFAULT_SHORTCUT) {
        trigger = super::PORTAL_DEFAULT_TRIGGER.to_string();
    }
    Ok(trigger)
}

pub(super) fn portal_error_was_denial(error: &ashpd::Error) -> bool {
    matches!(
        error,
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
            | ashpd::Error::Portal(ashpd::PortalError::Cancelled(_))
            | ashpd::Error::Portal(ashpd::PortalError::NotAllowed(_))
    )
}
