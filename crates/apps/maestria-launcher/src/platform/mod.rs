#[cfg(not(target_os = "linux"))]
use crate::{catalog::AppEntry, errors::LauncherError, model::SearchResult};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    DisplayBackend, apply_activation_token, decorate_results, display_backend, enumerate_apps,
    install_monitor, launch_app,
};

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    X11,
    Wayland,
    Unavailable,
}

#[cfg(not(target_os = "linux"))]
pub fn enumerate_apps() -> Result<Vec<AppEntry>, LauncherError> {
    Err(LauncherError::platform_unavailable(
        "application discovery is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn launch_app(_window: &tauri::WebviewWindow, _desktop_id: &str) -> Result<(), LauncherError> {
    Err(LauncherError::platform_unavailable(
        "application launching is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn decorate_results(
    _window: &tauri::WebviewWindow,
    _apps: &[AppEntry],
    _results: &mut [SearchResult],
) {
}

#[cfg(not(target_os = "linux"))]
pub fn install_monitor<F>(_on_changed: F) -> Result<(), LauncherError>
where
    F: Fn() + 'static,
{
    Err(LauncherError::platform_unavailable(
        "application change monitoring is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn display_backend(_window: &tauri::WebviewWindow) -> DisplayBackend {
    DisplayBackend::Unavailable
}

#[cfg(not(target_os = "linux"))]
pub fn apply_activation_token(
    _window: &tauri::WebviewWindow,
    _token: &str,
) -> Result<(), LauncherError> {
    Err(LauncherError::platform_unavailable(
        "activation tokens are unavailable on this platform",
    ))
}
