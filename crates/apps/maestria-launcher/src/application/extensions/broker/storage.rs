use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use maestria_extensions::{CapabilitySuccess, StorageOperation};

const MAX_STORAGE_VALUE_BYTES: usize = 16_384;
const MAX_STORAGE_KEYS: usize = 128;
const MAX_STORAGE_TOTAL_BYTES: u64 = 1_048_576;
static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);

static STORAGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy)]
pub(super) enum StorageError {
    NotFound,
    InvalidRequest,
    Unavailable,
    Failed,
}

pub(super) fn execute(
    storage_root: &Path,
    extension_id: &str,
    operation: StorageOperation,
    key: &str,
    value: Option<&str>,
) -> Result<CapabilitySuccess, StorageError> {
    let _storage_guard = STORAGE_LOCK.lock().map_err(|_| StorageError::Failed)?;
    if matches!(operation, StorageOperation::Set)
        && value.is_none_or(|value| value.len() > MAX_STORAGE_VALUE_BYTES)
    {
        return Err(StorageError::InvalidRequest);
    }
    let key = encoded_key(key)?;
    match operation {
        StorageOperation::Get => {
            let value = get_value(storage_root, extension_id, &key)?;
            Ok(CapabilitySuccess::Storage {
                ok: true,
                operation,
                value,
                completed: None,
            })
        }
        StorageOperation::Set => {
            let value = value.ok_or(StorageError::InvalidRequest)?;
            let directory = create_extension_directory(storage_root, extension_id)?;
            check_quota(&directory, &directory.join(&key), value.len())?;
            atomic_set(&directory, &directory.join(&key), value.as_bytes())?;
            Ok(CapabilitySuccess::Storage {
                ok: true,
                operation,
                value: None,
                completed: Some(true),
            })
        }
        StorageOperation::Delete => {
            let directory = existing_extension_directory(storage_root, extension_id)?
                .ok_or(StorageError::NotFound)?;
            fs::remove_file(directory.join(&key)).map_err(map_io)?;
            Ok(CapabilitySuccess::Storage {
                ok: true,
                operation,
                value: None,
                completed: Some(true),
            })
        }
    }
}

fn get_value(
    storage_root: &Path,
    extension_id: &str,
    key: &str,
) -> Result<Option<String>, StorageError> {
    let Some(directory) = existing_extension_directory(storage_root, extension_id)? else {
        return Ok(None);
    };
    let mut file = match open_readonly(&directory.join(key)) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io(error)),
    };
    let metadata = file.metadata().map_err(map_io)?;
    if !metadata.is_file() || metadata.len() > MAX_STORAGE_VALUE_BYTES as u64 {
        return Err(StorageError::Failed);
    }
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_STORAGE_VALUE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(map_io)?;
    if bytes.len() > MAX_STORAGE_VALUE_BYTES {
        return Err(StorageError::Failed);
    }
    let value = String::from_utf8(bytes).map_err(|_| StorageError::Failed)?;
    Ok(Some(value))
}

fn check_quota(directory: &Path, target: &Path, value_bytes: usize) -> Result<(), StorageError> {
    let target_name = target.file_name().ok_or(StorageError::Failed)?;
    let mut entry_count = 0;
    let mut total_bytes = 0_u64;
    let mut replaced_bytes = 0_u64;
    let mut target_exists = false;
    for entry in fs::read_dir(directory)
        .map_err(map_io)?
        .take(MAX_STORAGE_KEYS + 1)
    {
        let entry = entry.map_err(map_io)?;
        entry_count += 1;
        if entry_count > MAX_STORAGE_KEYS {
            return Err(StorageError::InvalidRequest);
        }
        if !entry.file_type().map_err(map_io)?.is_file() {
            return Err(StorageError::Failed);
        }
        let metadata = entry.metadata().map_err(map_io)?;
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or(StorageError::Failed)?;
        if entry.file_name().as_os_str() == target_name {
            replaced_bytes = metadata.len();
            target_exists = true;
        }
    }
    let new_count = if target_exists {
        entry_count
    } else {
        entry_count + 1
    };
    let value_bytes = u64::try_from(value_bytes).map_err(|_| StorageError::InvalidRequest)?;
    let total_without_target = total_bytes
        .checked_sub(replaced_bytes)
        .ok_or(StorageError::Failed)?;
    let new_total = total_without_target
        .checked_add(value_bytes)
        .ok_or(StorageError::InvalidRequest)?;
    if new_count > MAX_STORAGE_KEYS || new_total > MAX_STORAGE_TOTAL_BYTES {
        return Err(StorageError::InvalidRequest);
    }
    Ok(())
}

fn existing_extension_directory(
    storage_root: &Path,
    extension_id: &str,
) -> Result<Option<PathBuf>, StorageError> {
    let root_metadata = match fs::symlink_metadata(storage_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io(error)),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(StorageError::Unavailable);
    }
    let directory = storage_root.join(extension_id);
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io(error)),
    };
    validate_private_directory(&metadata)?;
    Ok(Some(directory))
}

fn create_extension_directory(
    storage_root: &Path,
    extension_id: &str,
) -> Result<PathBuf, StorageError> {
    fs::create_dir_all(storage_root).map_err(map_io)?;
    let root_metadata = fs::symlink_metadata(storage_root).map_err(map_io)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(StorageError::Unavailable);
    }
    let directory = storage_root.join(extension_id);
    match create_private_directory(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(map_io(error)),
    }
    let metadata = fs::symlink_metadata(&directory).map_err(map_io)?;
    validate_private_directory(&metadata)?;
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(target_os = "linux"))]
fn create_private_directory(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "private extension storage is unavailable on this platform",
    ))
}

fn validate_private_directory(metadata: &fs::Metadata) -> Result<(), StorageError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StorageError::Unavailable);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(StorageError::Unavailable);
        }
    }
    Ok(())
}

fn atomic_set(directory: &Path, target: &Path, value: &[u8]) -> Result<(), StorageError> {
    let temporary = temporary_path(directory, target)?;
    let mut file = create_private_file(&temporary).map_err(map_io)?;
    if let Err(error) = file.write_all(value).and_then(|()| file.sync_all()) {
        drop(file);
        remove_temporary(&temporary, error)
    } else {
        drop(file);
        if let Err(error) = fs::rename(&temporary, target) {
            return remove_temporary(&temporary, error);
        }
        File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(map_io)
    }
}

fn temporary_path(directory: &Path, target: &Path) -> Result<PathBuf, StorageError> {
    let filename = target.file_name().ok_or(StorageError::Failed)?;
    let sequence = NEXT_TEMP_FILE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| StorageError::Failed)?;
    Ok(directory.join(format!(
        ".{}.tmp-{}-{sequence}",
        filename.to_string_lossy(),
        std::process::id()
    )))
}

#[cfg(target_os = "linux")]
fn create_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .mode(0o600);
    options.open(path)
}

#[cfg(not(target_os = "linux"))]
fn create_private_file(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "private extension storage is unavailable on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn open_readonly(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options.open(path)
}

#[cfg(not(target_os = "linux"))]
fn open_readonly(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "private extension storage is unavailable on this platform",
    ))
}

fn remove_temporary(path: &Path, cause: io::Error) -> Result<(), StorageError> {
    match fs::remove_file(path) {
        Ok(()) => Err(map_io(cause)),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(map_io(cause)),
        Err(_) => Err(StorageError::Failed),
    }
}

fn encoded_key(key: &str) -> Result<String, StorageError> {
    if key.is_empty() || key.len() > 64 {
        return Err(StorageError::InvalidRequest);
    }
    let mut encoded = String::with_capacity(key.len() * 2);
    for byte in key.bytes() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").map_err(|_| StorageError::Failed)?;
    }
    Ok(encoded)
}

fn map_io(error: io::Error) -> StorageError {
    match error.kind() {
        ErrorKind::NotFound => StorageError::NotFound,
        ErrorKind::InvalidInput | ErrorKind::InvalidData => StorageError::InvalidRequest,
        ErrorKind::Unsupported => StorageError::Unavailable,
        _ => StorageError::Failed,
    }
}
