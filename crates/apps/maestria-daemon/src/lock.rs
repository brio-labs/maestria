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
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let token = format!("{}:{nonce}", std::process::id());
            if let Err(error) = writeln!(file, "{token}") {
                let _ = fs::remove_file(&path);
                return Err(error)
                    .with_context(|| format!("write instance lock {}", path.display()));
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
                        let _ = fs::remove_file(&quarantine);
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
    .map_err(|_| anyhow!("timed out waiting for instance write lock"))?
}

fn lock_owner_is_dead(path: &PathBuf) -> bool {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => {
            return fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                .is_ok_and(|age| age > Duration::from_secs(30));
        }
    };
    let pid_text = contents
        .trim()
        .split_once(':')
        .map_or(contents.trim(), |(pid, _)| pid);
    let Ok(pid) = pid_text.parse::<u32>() else {
        return fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .is_ok_and(|age| age > Duration::from_secs(30));
    };
    #[cfg(target_os = "linux")]
    {
        !PathBuf::from(format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}
