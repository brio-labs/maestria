use std::fs::File;
#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::BundleError;
use super::persistence;

static STORE_LOCK: Mutex<()> = Mutex::new(());
const STORE_LOCK_FILE: &str = ".bundle-store.lock";

pub(super) fn store_lock() -> Result<MutexGuard<'static, ()>, BundleError> {
    STORE_LOCK.lock().map_err(|_| BundleError::LockPoisoned)
}

pub(super) struct InterprocessStoreLock {
    _file: File,
    root_directory: File,
}

impl InterprocessStoreLock {
    pub(super) fn root_path(&self, _fallback: &Path) -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;

            PathBuf::from(format!(
                "/proc/self/fd/{}/.",
                self.root_directory.as_raw_fd()
            ))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = &self.root_directory;
            _fallback.to_path_buf()
        }
    }
}

pub(super) fn interprocess_store_lock(
    root: &Path,
) -> Result<Option<InterprocessStoreLock>, BundleError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        const O_CLOEXEC: i32 = 0o2000000;
        const O_DIRECTORY: i32 = 0o200000;
        const O_NOFOLLOW: i32 = 0o400000;
        const O_NONBLOCK: i32 = 0o4000;

        // Pin the root directory and address its children through this fd so a swapped path
        // cannot redirect the lock or writes to a different store.
        let mut root_options = OpenOptions::new();
        root_options
            .read(true)
            .custom_flags(O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW);
        let root_directory = match root_options.open(root) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(persistence::io_error(root, source)),
        };
        let root_metadata = root_directory
            .metadata()
            .map_err(|source| persistence::io_error(root, source))?;
        if !root_metadata.is_dir() || root_metadata.mode() & 0o077 != 0 {
            return Err(BundleError::InvalidState(
                "extension store root must be a private real directory",
            ));
        }

        let directory_fd_path =
            PathBuf::from(format!("/proc/self/fd/{}", root_directory.as_raw_fd()));
        let resolved_root = std::fs::canonicalize(&directory_fd_path)
            .map_err(|source| persistence::io_error(root, source))?;
        if resolved_root.as_path() != root {
            return Err(BundleError::InvalidState(
                "extension store root changed before lock acquisition",
            ));
        }

        let lock_path = root.join(STORE_LOCK_FILE);
        let proc_lock_path = directory_fd_path.join(STORE_LOCK_FILE);
        let mut lock_options = OpenOptions::new();
        lock_options
            .read(true)
            .write(true)
            .create(true)
            .custom_flags(O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK)
            .mode(0o600);
        let file = lock_options
            .open(&proc_lock_path)
            .map_err(|source| persistence::io_error(&lock_path, source))?;
        let metadata = file
            .metadata()
            .map_err(|source| persistence::io_error(&lock_path, source))?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != root_metadata.uid()
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(BundleError::InvalidState(
                "extension store lock must be a private single-link regular file",
            ));
        }
        file.lock()
            .map_err(|source| persistence::io_error(&lock_path, source))?;
        Ok(Some(InterprocessStoreLock {
            _file: file,
            root_directory,
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        match std::fs::symlink_metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(persistence::io_error(root, source)),
            Ok(_) => Err(persistence::io_error(
                &root.join(STORE_LOCK_FILE),
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "secure interprocess extension store locking is unavailable on this platform",
                ),
            )),
        }
    }
}
