use std::path::{Path, PathBuf};

use crate::errors::LauncherError;

pub(super) fn copy_text(value: &str) -> Result<(), LauncherError> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return copy_wayland_text(value);
        }
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|error| LauncherError::clipboard_failed(error.to_string()))?;
        clipboard
            .set_text(value.to_string())
            .map_err(|error| LauncherError::clipboard_failed(error.to_string()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = value;
        Err(LauncherError::platform_unavailable(
            "Clipboard integration is unavailable on this platform",
        ))
    }
}

#[cfg(target_os = "linux")]
fn copy_wayland_text(value: &str) -> Result<(), LauncherError> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::process::{Command, Stdio};
    use std::time::Duration;
    use tokio::time::Instant as MonotonicInstant;

    const MAX_CLIPBOARD_BYTES: usize = 65_536;
    if value.len() > MAX_CLIPBOARD_BYTES {
        return Err(LauncherError::clipboard_failed(
            "Clipboard text exceeds the 64 KiB Wayland limit",
        ));
    }
    let mut child = Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| LauncherError::clipboard_failed(format!("start wl-copy: {error}")))?;
    let deadline = MonotonicInstant::now() + Duration::from_millis(500);
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| LauncherError::clipboard_failed("wl-copy stdin is unavailable"))
        .and_then(|mut stdin| {
            let fd = stdin.as_raw_fd();
            // SAFETY: stdin owns this valid file descriptor for both fcntl calls.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags == -1
                || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
            {
                return Err(LauncherError::clipboard_failed(format!(
                    "set wl-copy stdin nonblocking: {}",
                    std::io::Error::last_os_error()
                )));
            }
            let mut remaining = value.as_bytes();
            while !remaining.is_empty() {
                if MonotonicInstant::now() >= deadline {
                    return Err(LauncherError::clipboard_failed(
                        "wl-copy did not accept clipboard text within 500 ms",
                    ));
                }
                match stdin.write(remaining) {
                    Ok(0) => {
                        return Err(LauncherError::clipboard_failed(
                            "wl-copy stdin closed early",
                        ));
                    }
                    Ok(written) => remaining = &remaining[written..],
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => {
                        return Err(LauncherError::clipboard_failed(format!(
                            "write wl-copy: {error}"
                        )));
                    }
                }
            }
            Ok(())
        });
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(LauncherError::clipboard_failed(format!(
                    "wl-copy exited with {status}"
                )));
            }
            Ok(None) if MonotonicInstant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LauncherError::clipboard_failed(
                    "wl-copy did not accept clipboard text within 500 ms",
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LauncherError::clipboard_failed(format!(
                    "wait for wl-copy: {error}"
                )));
            }
        }
    }
}

pub(super) fn validate_selected_path(path: &Path) -> Result<(), LauncherError> {
    if !path.is_absolute() || path.to_str().is_none() || !path.is_file() {
        return Err(LauncherError::file_unavailable(
            "The selected local file is unavailable or has an invalid path",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) async fn choose_file(
    parent: Option<ashpd::WindowIdentifier>,
) -> Result<Option<PathBuf>, LauncherError> {
    use ashpd::desktop::file_chooser::OpenFileRequest;

    let request = OpenFileRequest::default()
        .title("Open a local file")
        .accept_label("Open")
        .modal(true)
        .identifier(parent)
        .send()
        .await
        .map_err(|error| LauncherError::file_unavailable(error.to_string()))?;
    let response = match request.response() {
        Ok(response) => response,
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => return Ok(None),
        Err(error) => return Err(LauncherError::file_unavailable(error.to_string())),
    };
    let Some(uri) = response.uris().first() else {
        return Ok(None);
    };
    uri.to_file_path()
        .map(Some)
        .map_err(|()| LauncherError::file_unavailable("The chooser returned a non-local file"))
}

#[cfg(not(target_os = "linux"))]
pub(super) async fn choose_file(_parent: Option<()>) -> Result<Option<PathBuf>, LauncherError> {
    Err(LauncherError::platform_unavailable(
        "The native file chooser is unavailable on this platform",
    ))
}

pub(super) fn system_dark_mode() -> bool {
    if std::env::var("GTK_THEME").is_ok_and(|theme| theme.to_ascii_lowercase().contains("dark")) {
        return true;
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("prefer-dark")
        })
}

pub(super) fn config_dir() -> Result<PathBuf, String> {
    if let Some(directory) = std::env::var_os("XDG_CONFIG_HOME") {
        let directory = PathBuf::from(directory);
        if directory.is_absolute() {
            return Ok(directory.join("io.github.briolabs.Maestria.Launcher"));
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config/io.github.briolabs.Maestria.Launcher"))
        .ok_or_else(|| "the user configuration directory is unavailable".to_string())
}
