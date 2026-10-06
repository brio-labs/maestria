use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::time::Instant as MonotonicInstant;

use serde::{Deserialize, Serialize};

use super::{HttpGrantError, HttpGrantPolicy};

const MAX_GRANTS: usize = 256;
const MAX_RECORD_BYTES: u64 = 32 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 2048;
#[cfg(unix)]
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredGrant {
    pub(super) handle: String,
    pub(super) policy: HttpGrantPolicy,
    pub(super) created_at: u64,
    pub(super) expires_at: u64,
    pub(super) revoked: bool,
}

pub(super) fn preflight(root: &Path) -> Result<(), HttpGrantError> {
    with_lock(root, true, |directory| {
        if read_records_locked(directory)?.len() >= MAX_GRANTS {
            return Err(HttpGrantError::Failed);
        }
        Ok(())
    })
}

pub(super) fn insert(root: &Path, record: &StoredGrant) -> Result<(), HttpGrantError> {
    with_lock(root, true, |directory| {
        let records = read_records_locked(directory)?;
        if records.len() >= MAX_GRANTS || records.iter().any(|item| item.handle == record.handle) {
            return Err(HttpGrantError::Failed);
        }
        let path = record_path(directory, &record.handle)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(HttpGrantError::Failed);
        }
        write_record(directory, &path, record)
    })
}

pub(super) fn read_all(root: &Path) -> Result<Vec<StoredGrant>, HttpGrantError> {
    if !ensure_root(root, false)? {
        return Ok(Vec::new());
    }
    with_lock(root, false, read_records_locked)
}

pub(super) fn read_one(root: &Path, handle: &str) -> Result<Option<StoredGrant>, HttpGrantError> {
    let path = record_path(root, handle)?;
    if !ensure_root(root, false)? {
        return Ok(None);
    }
    with_lock(root, false, |_| {
        let record = read_record(&path)?;
        if record
            .as_ref()
            .is_some_and(|record| record.handle != handle)
        {
            return Err(HttpGrantError::Failed);
        }
        Ok(record)
    })
}

pub(super) fn revoke(root: &Path, handle: &str) -> Result<(), HttpGrantError> {
    let path = record_path(root, handle)?;
    if !ensure_root(root, false)? {
        return Err(HttpGrantError::InvalidHandle);
    }
    with_lock(root, false, |_| {
        let mut record = read_record(&path)?.ok_or(HttpGrantError::InvalidHandle)?;
        if record.handle != handle {
            return Err(HttpGrantError::Failed);
        }
        if !record.revoked {
            record.revoked = true;
            write_record(root, &path, &record)?;
        }
        Ok(())
    })
}

fn with_lock<T>(
    root: &Path,
    create_root: bool,
    operation: impl FnOnce(&Path) -> Result<T, HttpGrantError>,
) -> Result<T, HttpGrantError> {
    if !ensure_root(root, create_root)? {
        return Err(HttpGrantError::Failed);
    }
    #[cfg(unix)]
    let _lock = acquire_lock(root)?;
    operation(root)
}

fn record_path(root: &Path, handle: &str) -> Result<PathBuf, HttpGrantError> {
    if handle.is_empty()
        || handle.len() > 128
        || !handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(HttpGrantError::InvalidHandle);
    }
    Ok(root.join(format!("grant-{handle}.json")))
}

fn read_records_locked(root: &Path) -> Result<Vec<StoredGrant>, HttpGrantError> {
    let mut records = Vec::new();
    let entries = fs::read_dir(root).map_err(|_| HttpGrantError::Failed)?;
    let mut visited = 0usize;
    for entry in entries {
        visited = visited.checked_add(1).ok_or(HttpGrantError::Failed)?;
        if visited > MAX_DIRECTORY_ENTRIES {
            return Err(HttpGrantError::Failed);
        }
        let entry = entry.map_err(|_| HttpGrantError::Failed)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(handle) = name
            .strip_prefix("grant-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let path = record_path(root, handle).map_err(|_| HttpGrantError::Failed)?;
        if !entry
            .file_type()
            .map_err(|_| HttpGrantError::Failed)?
            .is_file()
        {
            return Err(HttpGrantError::Failed);
        }
        let record = read_record(&path)?.ok_or(HttpGrantError::Failed)?;
        validate_record(&record)?;
        if record.handle != handle {
            return Err(HttpGrantError::Failed);
        }
        records.push(record);
        if records.len() > MAX_GRANTS {
            return Err(HttpGrantError::Failed);
        }
    }
    Ok(records)
}

fn read_record(path: &Path) -> Result<Option<StoredGrant>, HttpGrantError> {
    let file = match open_private_file(path) {
        Ok(file) => file,
        Err(HttpGrantError::InvalidHandle) => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata().map_err(|_| HttpGrantError::Failed)?;
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(HttpGrantError::Failed);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| HttpGrantError::Failed)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(HttpGrantError::Failed);
    }
    let record: StoredGrant = serde_json::from_slice(&bytes).map_err(|_| HttpGrantError::Failed)?;
    validate_record(&record)?;
    Ok(Some(record))
}

fn validate_record(record: &StoredGrant) -> Result<(), HttpGrantError> {
    super::validate_policy(&record.policy).map_err(|_| HttpGrantError::Failed)?;
    let expiry = record
        .created_at
        .checked_add(record.policy.expires_in_seconds)
        .ok_or(HttpGrantError::Failed)?;
    if record.handle.is_empty()
        || record.handle.len() > 128
        || !record
            .handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || record.created_at == 0
        || record.expires_at != expiry
    {
        return Err(HttpGrantError::Failed);
    }
    Ok(())
}

fn write_record(root: &Path, path: &Path, record: &StoredGrant) -> Result<(), HttpGrantError> {
    let bytes = serde_json::to_vec(record).map_err(|_| HttpGrantError::Failed)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(HttpGrantError::Failed);
    }
    if fs::symlink_metadata(path).is_ok() {
        let _ = open_private_file(path)?;
    }
    let (temporary_path, mut temporary) = create_temporary_file(root, path)?;
    if temporary
        .write_all(&bytes)
        .and_then(|()| temporary.sync_all())
        .is_err()
    {
        drop(temporary);
        let _ = fs::remove_file(&temporary_path);
        return Err(HttpGrantError::Failed);
    }
    drop(temporary);
    if fs::rename(&temporary_path, path).is_err() {
        let _ = fs::remove_file(&temporary_path);
        return Err(HttpGrantError::Failed);
    }
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| HttpGrantError::Failed)
}

fn create_temporary_file(root: &Path, target: &Path) -> Result<(PathBuf, File), HttpGrantError> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(HttpGrantError::Failed)?;
    for _ in 0..32 {
        let number = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".{name}.tmp-{}-{number}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(HttpGrantError::Failed),
        }
    }
    Err(HttpGrantError::Failed)
}

#[cfg(unix)]
fn open_private_file(path: &Path) -> Result<File, HttpGrantError> {
    let mut options = OpenOptions::new();
    options.read(true).write(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            HttpGrantError::InvalidHandle
        } else {
            HttpGrantError::Failed
        }
    })?;
    let metadata = file.metadata().map_err(|_| HttpGrantError::Failed)?;
    validate_private_file(&metadata)?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_private_file(_: &Path) -> Result<File, HttpGrantError> {
    Err(HttpGrantError::Unavailable)
}

fn ensure_root(root: &Path, create: bool) -> Result<bool, HttpGrantError> {
    if !root.is_absolute() {
        return Err(HttpGrantError::Failed);
    }
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            validate_private_directory(&metadata)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
                options.open(root).map_err(|_| HttpGrantError::Failed)?;
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(root).map_err(|_| HttpGrantError::Failed)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(root, fs::Permissions::from_mode(0o700))
                    .map_err(|_| HttpGrantError::Failed)?;
            }
            let metadata = fs::symlink_metadata(root).map_err(|_| HttpGrantError::Failed)?;
            validate_private_directory(&metadata)?;
            Ok(true)
        }
        Err(_) => Err(HttpGrantError::Failed),
    }
}

#[cfg(unix)]
fn validate_private_directory(metadata: &fs::Metadata) -> Result<(), HttpGrantError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !metadata.file_type().is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(HttpGrantError::Failed);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_directory(_: &fs::Metadata) -> Result<(), HttpGrantError> {
    Err(HttpGrantError::Unavailable)
}

#[cfg(unix)]
fn validate_private_file(metadata: &fs::Metadata) -> Result<(), HttpGrantError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !metadata.file_type().is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(HttpGrantError::Failed);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file(_: &fs::Metadata) -> Result<(), HttpGrantError> {
    Err(HttpGrantError::Unavailable)
}

#[cfg(unix)]
fn acquire_lock(root: &Path) -> Result<File, HttpGrantError> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = root.join(".lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).mode(0o600);
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(&path).map_err(|_| HttpGrantError::Failed)?;
    let metadata = file.metadata().map_err(|_| HttpGrantError::Failed)?;
    validate_private_file(&metadata)?;
    if metadata.len() != 0 {
        return Err(HttpGrantError::Failed);
    }
    let deadline = MonotonicInstant::now() + LOCK_TIMEOUT;
    loop {
        let result = unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if result == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) || MonotonicInstant::now() >= deadline
        {
            return Err(HttpGrantError::Failed);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
