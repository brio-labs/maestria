use std::io::Write;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::LauncherError;

const MAX_BYTES: usize = 65_536;
const READ_DEADLINE: Duration = Duration::from_millis(500);
const CLEANUP_DEADLINE: Duration = Duration::from_millis(100);
#[cfg(target_os = "linux")]
const HELPER_ADDRESS_SPACE_LIMIT: libc::rlim_t = 256 * 1024 * 1024;

/// Export text for the launcher's explicit-capture child process only.
///
/// This worker does not retain clipboard contents or initialize the launcher UI.
pub fn export_clipboard_text() -> Result<(), LauncherError> {
    #[cfg(target_os = "linux")]
    {
        install_clipboard_memory_limit()?;
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|_| LauncherError::clipboard_failed("The clipboard is unavailable"))?;
        let text = clipboard
            .get_text()
            .map_err(|_| LauncherError::clipboard_failed("No clipboard text is available"))?;
        if text.len() > MAX_BYTES {
            return Err(LauncherError::clipboard_failed(
                "Clipboard text exceeds 64 KiB",
            ));
        }
        std::io::stdout()
            .lock()
            .write_all(text.as_bytes())
            .map_err(|_| LauncherError::clipboard_failed("Clipboard transfer failed"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(LauncherError::platform_unavailable(
            "Clipboard capture is unavailable on this platform",
        ))
    }
}

#[cfg(target_os = "linux")]
fn install_clipboard_memory_limit() -> Result<(), LauncherError> {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limits) } != 0 {
        return Err(LauncherError::clipboard_failed(format!(
            "The clipboard worker memory limit could not be read: {}",
            std::io::Error::last_os_error()
        )));
    }

    // Limit both soft and hard caps in this helper process, without raising an
    // inherited restriction. Lowering the hard cap also prevents later raising
    // the soft limit back above the intended bound.
    limits.rlim_max = limits.rlim_max.min(HELPER_ADDRESS_SPACE_LIMIT);
    limits.rlim_cur = limits.rlim_cur.min(limits.rlim_max);
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limits) } != 0 {
        return Err(LauncherError::clipboard_failed(format!(
            "The clipboard worker memory limit could not be installed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

pub(crate) async fn read_text() -> Result<String, LauncherError> {
    let executable = std::env::current_exe().map_err(|_| {
        LauncherError::clipboard_failed("The clipboard worker executable is unavailable")
    })?;
    let mut child = Command::new(executable)
        .arg("--read-clipboard")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| LauncherError::clipboard_failed("The clipboard worker could not start"))?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            stop_worker(&mut child).await?;
            return Err(LauncherError::clipboard_failed(
                "Clipboard transfer is unavailable",
            ));
        }
    };
    let mut bytes = Vec::new();
    let operation = async {
        stdout
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| LauncherError::clipboard_failed("Clipboard transfer failed"))?;
        if bytes.len() > MAX_BYTES {
            return Err(LauncherError::clipboard_failed(
                "Clipboard text exceeds 64 KiB",
            ));
        }
        let status = child.wait().await.map_err(|_| {
            LauncherError::clipboard_failed("The clipboard worker could not be reaped")
        })?;
        if !status.success() {
            return Err(LauncherError::clipboard_failed(
                "Clipboard worker exited unsuccessfully",
            ));
        }
        Ok(())
    };
    match tokio::time::timeout(READ_DEADLINE, operation).await {
        Ok(Ok(())) => String::from_utf8(bytes)
            .map_err(|_| LauncherError::clipboard_failed("Clipboard text is not valid UTF-8")),
        Ok(Err(error)) => {
            stop_worker(&mut child).await?;
            Err(error)
        }
        Err(_) => {
            stop_worker(&mut child).await?;
            Err(LauncherError::clipboard_failed(
                "Clipboard capture exceeded 500 ms; nothing was saved",
            ))
        }
    }
}

async fn stop_worker(child: &mut tokio::process::Child) -> Result<(), LauncherError> {
    let exited = child.try_wait().map_err(|_| {
        LauncherError::clipboard_failed("Clipboard worker status could not be read")
    })?;
    if exited.is_some() {
        return Ok(());
    }

    if let Err(error) = child.start_kill() {
        let exited = child.try_wait().map_err(|_| {
            LauncherError::clipboard_failed("Clipboard worker status could not be read")
        })?;
        if exited.is_some() {
            return Ok(());
        }
        return Err(LauncherError::clipboard_failed(format!(
            "Clipboard worker termination failed: {error}"
        )));
    }

    match tokio::time::timeout(CLEANUP_DEADLINE, child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Err(LauncherError::clipboard_failed(
            "Clipboard worker could not be reaped after termination",
        )),
        Err(_) => Err(LauncherError::clipboard_failed(
            "Clipboard worker did not exit within 100 ms after termination",
        )),
    }
}
