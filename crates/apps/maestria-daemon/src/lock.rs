//! Instance write-lock management for Maestria daemon.
//!
//! Lock files prevent concurrent instance modifications from multiple
//! daemon or CLI processes targeting the same instance directory.

use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use tokio::time::timeout;

/// Guard token that represents ownership of the instance write lock.
/// Dropping the guard releases the lock by removing the lock file
/// (only if this process still owns it).
pub struct InstanceWriteLock {
    path: PathBuf,
    token: String,
}

impl Drop for InstanceWriteLock {
    fn drop(&mut self) {
        let owned =
            fs::read_to_string(&self.path).is_ok_and(|contents| contents.trim() == self.token);
        if owned && let Err(error) = fs::remove_file(&self.path) {
            tracing::warn!(path = %self.path.display(), %error, "failed to release instance write lock");
        }
    }
}

/// Attempt to acquire the instance write lock without waiting.
///
/// Returns `Ok(None)` if another process holds the lock and appears alive.
/// Returns `Ok(Some(lock))` on success. Returns an error on I/O failure.
pub fn try_acquire(layout: &InstanceLayout) -> Result<Option<InstanceWriteLock>> {
    let path = layout.system_dir.join("instance-write.lock");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            let token = lock_token();
            if let Err(error) = writeln!(file, "{token}") {
                let write_error =
                    anyhow!(error).context(format!("write instance lock {}", path.display()));
                return match fs::remove_file(&path) {
                    Ok(()) => Err(write_error),
                    Err(cleanup_error) => Err(write_error.context(format!(
                        "failed to remove partial lock {}: {cleanup_error}",
                        path.display()
                    ))),
                };
            }
            Ok(Some(InstanceWriteLock { path, token }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !lock_owner_is_dead(&path) {
                return Ok(None);
            }
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let quarantine = path.with_extension(format!("stale.{}.{}", std::process::id(), nonce));
            match fs::hard_link(&path, &quarantine) {
                Ok(()) => match fs::remove_file(&path) {
                    Ok(()) => {
                        fs::remove_file(&quarantine).with_context(|| {
                            format!("remove stale instance lock {}", quarantine.display())
                        })?;
                        try_acquire(layout)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if let Err(cleanup_error) = fs::remove_file(&quarantine) {
                            tracing::warn!(
                                path = %quarantine.display(),
                                %cleanup_error,
                                "failed to remove orphaned quarantine lock"
                            );
                        }
                        try_acquire(layout)
                    }
                    Err(error) => Err(error)
                        .with_context(|| format!("remove stale instance lock {}", path.display())),
                },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
                Err(error) => Err(error)
                    .with_context(|| format!("quarantine instance lock {}", path.display())),
            }
        }
        Err(error) => {
            Err(error).with_context(|| format!("create instance lock {}", path.display()))
        }
    }
}

/// Acquire the instance write lock, retrying with a 5-second timeout.
///
/// # Cancellation
/// The future is bounded by a 5-second timeout; if the timeout fires first,
/// the returned error is `timed out waiting for instance write lock`. Dropping
/// the future while it waits abandons the attempt — no lock is left behind
/// because acquisition only succeeds after the lock file is written.
pub async fn acquire(layout: &InstanceLayout) -> Result<InstanceWriteLock> {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Some(lock) = try_acquire(layout)? {
                return Ok(lock);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|error| {
        anyhow!(
            "timed out waiting for instance write lock (another process, e.g. a running \
             daemon, owns the instance; stop the daemon or use the Studio): {error}"
        )
    })?
}

/// Decide whether a recorded lock holder is gone.
///
/// The lock token records the holder's pid and its process start ticks.
/// Comparing start ticks separates a genuinely alive holder from a reused
/// pid: after a crash, the kernel can hand the pid to an unrelated process,
/// and a pure `/proc/{pid}` existence check would then treat the stale lock
/// as live indefinitely (the instance stays locked until manual cleanup).
///
/// Malformed or legacy files (no usable discriminator) fall back to the
/// 30-second mtime staleness horizon.
fn lock_owner_is_dead(path: &PathBuf) -> bool {
    let stale_by_age = || {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .is_ok_and(|age| age > Duration::from_secs(30))
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return stale_by_age();
    };
    let Some((pid_text, recorded)) = contents.trim().split_once(':') else {
        return stale_by_age();
    };
    let Ok(pid) = pid_text.parse::<u32>() else {
        return stale_by_age();
    };
    #[cfg(target_os = "linux")]
    {
        let recorded_ticks = recorded
            .strip_prefix('t')
            .and_then(|value| value.parse::<u64>().ok());
        match (recorded_ticks, process_start_ticks_for(pid)) {
            // Same process incarnation: the holder is alive.
            (Some(recorded), Some(live)) => recorded != live,
            // Reused pid (no /proc entry for it): the holder is gone.
            (_, None) => !PathBuf::from(format!("/proc/{pid}")).exists(),
            // Legacy wall-clock nonce token: trust pid liveness.
            (None, Some(_)) => false,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, recorded);
        stale_by_age()
    }
}

#[cfg(target_os = "linux")]
fn process_start_ticks_for(pid: u32) -> Option<u64> {
    // Field 22 of /proc/{pid}/stat is the start time in clock ticks. The
    // comm field (field 2) may contain spaces and parentheses, so parsing
    // starts after the last ')'; from there, state is field 3, making
    // starttime the 20th whitespace-separated token that follows.
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn process_start_ticks() -> Option<u64> {
    process_start_ticks_for(std::process::id())
}

/// Build this process's lock token: pid plus a discriminator that
/// identifies this exact process incarnation. Non-Linux platforms keep the
/// wall-clock nonce (liveness falls back to the mtime horizon there).
fn lock_token() -> String {
    #[cfg(target_os = "linux")]
    if let Some(ticks) = process_start_ticks() {
        return format!("{}:t{ticks}", std::process::id());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}:{nonce}", std::process::id())
}

#[cfg(all(test, target_os = "linux"))]
#[path = "lock_tests.rs"]
mod tests;
