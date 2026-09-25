#[cfg(not(target_os = "linux"))]
use crate::errors::LauncherError;
#[cfg(not(target_os = "linux"))]
use crate::model::{ShortcutConfigureAction, ShortcutControl, ShortcutState, ShortcutStatus};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::Shortcuts;

#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
pub struct Shortcuts {
    status: ShortcutStatus,
}

#[cfg(not(target_os = "linux"))]
impl Shortcuts {
    pub fn new(_activation: std::sync::mpsc::SyncSender<()>) -> Self {
        Self {
            status: unavailable_status("Global shortcuts are unavailable on this platform"),
        }
    }

    pub fn initialize(
        &self,
        _window: &slint::Window,
        _runtime: &tokio::runtime::Handle,
    ) -> Result<(), LauncherError> {
        Ok(())
    }

    pub fn status(&self) -> ShortcutStatus {
        self.status.clone()
    }

    /// Configure the platform's global shortcut.
    ///
    /// # Cancellation
    /// This fallback has no suspension point or platform side effect. It returns the current
    /// unavailable status on its first poll; cancellation before polling drops only that response.
    pub async fn configure(
        &self,
        _preferred: &str,
        _explicit: bool,
    ) -> Result<ShortcutStatus, LauncherError> {
        Ok(self.status())
    }

    /// Clear the platform's global shortcut.
    ///
    /// # Cancellation
    /// This fallback has no suspension point or platform side effect. It returns the current
    /// unavailable status on its first poll; cancellation before polling drops only that response.
    pub async fn clear(&self) -> Result<ShortcutStatus, LauncherError> {
        Ok(self.status())
    }

    pub fn shutdown(&self) -> Result<(), LauncherError> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn validate_accelerator(value: &str) -> Result<(), String> {
    linux::parse_hotkey(value)
        .map(|_| ())
        .map_err(|error| format!("invalid shortcut accelerator: {error}"))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn validate_accelerator(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains('\0') {
        Err("shortcut cannot be empty or contain NUL".to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn unavailable_status(message: &str) -> ShortcutStatus {
    ShortcutStatus {
        state: ShortcutState::Unavailable,
        description: "Global shortcut unavailable".to_string(),
        message: Some(message.to_string()),
        control: ShortcutControl::Unavailable,
        configure_action: ShortcutConfigureAction::Retry,
    }
}
