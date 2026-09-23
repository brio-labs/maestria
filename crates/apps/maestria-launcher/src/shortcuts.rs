#[cfg(not(target_os = "linux"))]
use crate::errors::LauncherError;
use crate::model::ShortcutStatus;
#[cfg(not(target_os = "linux"))]
use crate::model::{ShortcutConfigureAction, ShortcutControl, ShortcutState};
#[cfg(not(target_os = "linux"))]
use tauri::{AppHandle, WebviewWindow};

/// Result of an explicit or restored shortcut configuration request.
#[derive(Debug, Clone)]
pub struct ConfigureOutcome {
    pub status: ShortcutStatus,
    pub denied: bool,
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{Shortcuts, clear, configure, initialize, request_shutdown, shutdown_complete};

#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
pub struct Shortcuts;

#[cfg(not(target_os = "linux"))]
impl Shortcuts {
    pub fn new() -> Self {
        Self
    }

    pub fn status(&self) -> ShortcutStatus {
        unavailable_status("Global shortcuts are unavailable on this platform")
    }
}

#[cfg(not(target_os = "linux"))]
pub fn initialize(_app: &AppHandle, _window: &WebviewWindow) -> Result<(), LauncherError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
/// Configure the platform shortcut.
///
/// # Cancellation
///
/// Platform unavailability is returned as a status outcome; no native
/// registration work is started or left running.
pub async fn configure(
    _app: AppHandle,
    _preferred: String,
    _explicit: bool,
) -> Result<ConfigureOutcome, LauncherError> {
    Ok(ConfigureOutcome {
        status: unavailable_status("Global shortcuts are unavailable on this platform"),
        denied: false,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn clear(_app: &AppHandle) -> Result<(), LauncherError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn shutdown(_app: &AppHandle) -> Result<(), LauncherError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn request_shutdown(app: &AppHandle) -> Result<bool, LauncherError> {
    shutdown(app)?;
    app.exit(0);
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
pub fn shutdown_complete(_app: &AppHandle) -> bool {
    true
}

#[cfg(not(target_os = "linux"))]
fn unavailable_status(message: &str) -> ShortcutStatus {
    ShortcutStatus {
        state: ShortcutState::Unavailable,
        description: "Global shortcut unavailable".to_string(),
        message: Some(message.to_string()),
        control: ShortcutControl::Unavailable,
        configure_action: ShortcutConfigureAction::Retry,
    }
}
