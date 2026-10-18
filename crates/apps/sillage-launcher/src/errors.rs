use std::error::Error;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

impl LauncherError {
    pub fn new(code: &'static str, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            recoverable,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message, false)
    }

    pub fn stale_result(message: impl Into<String>) -> Self {
        Self::new("stale_result", message, true)
    }

    pub fn settings_failed(message: impl Into<String>) -> Self {
        Self::new("settings_failed", message, true)
    }

    pub fn file_unavailable(message: impl Into<String>) -> Self {
        Self::new("file_unavailable", message, true)
    }

    pub fn clipboard_failed(message: impl Into<String>) -> Self {
        Self::new("clipboard_failed", message, true)
    }

    pub fn platform_unavailable(message: impl Into<String>) -> Self {
        Self::new("platform_unavailable", message, true)
    }

    pub fn app_unavailable(message: impl Into<String>) -> Self {
        Self::new("app_unavailable", message, true)
    }

    pub fn launch_failed(message: impl Into<String>) -> Self {
        Self::new("launch_failed", message, true)
    }
}

impl std::fmt::Display for LauncherError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for LauncherError {}
