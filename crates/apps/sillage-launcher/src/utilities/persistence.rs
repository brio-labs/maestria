use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::errors::LauncherError;

use super::entry::{self, Entry};

pub(super) const UTILITIES_FILE: &str = "utilities.toml";
pub(super) const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
const SCHEMA_VERSION: u32 = 1;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct Persistence {
    path: PathBuf,
    disk_snapshot: Option<Vec<u8>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SerializedUtilities<'a, Entries: ?Sized> {
    schema_version: u32,
    entries: &'a Entries,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoadedUtilities {
    schema_version: u32,
    entries: Vec<Entry>,
}

impl Persistence {
    pub(super) fn load(
        directory: Result<PathBuf, String>,
    ) -> Result<(Self, Vec<Entry>), LauncherError> {
        let directory = directory.map_err(|message| {
            LauncherError::file_unavailable(format!(
                "Utilities cannot be persisted because the configuration directory is unavailable: {message}"
            ))
        })?;
        let path = directory.join(UTILITIES_FILE);
        let (entries, disk_snapshot) = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(LauncherError::file_unavailable(
                        "The utilities data path is not a regular file",
                    ));
                }
                if metadata.len() > MAX_FILE_BYTES as u64 {
                    return Err(LauncherError::file_unavailable(
                        "The utilities data file exceeds the supported size",
                    ));
                }
                let contents = read_utilities_file(&path).map_err(|error| {
                    LauncherError::file_unavailable(format!(
                        "Utilities data could not be read: {error}"
                    ))
                })?;
                let entries = parse_utilities(&contents).map_err(|message| {
                    LauncherError::file_unavailable(format!(
                        "Utilities data is invalid and was preserved: {message}"
                    ))
                })?;
                (entries, Some(contents))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Vec::new(), None),
            Err(error) => {
                return Err(LauncherError::file_unavailable(format!(
                    "Utilities data could not be inspected: {error}"
                )));
            }
        };

        Ok((
            Self {
                path,
                disk_snapshot,
            },
            entries,
        ))
    }

    pub(super) fn write<Entries: Serialize + ?Sized>(
        &mut self,
        entries: &Entries,
    ) -> Result<(), LauncherError> {
        let document = SerializedUtilities {
            schema_version: SCHEMA_VERSION,
            entries,
        };
        let contents = toml::to_string_pretty(&document).map_err(|error| {
            LauncherError::file_unavailable(format!(
                "Utilities data could not be serialized: {error}"
            ))
        })?;
        if contents.len() > MAX_FILE_BYTES {
            return Err(LauncherError::invalid_request(
                "The utilities store exceeds the supported file size",
            ));
        }
        atomic_replace(
            &self.path,
            contents.as_bytes(),
            self.disk_snapshot.as_deref(),
        )
        .map_err(|error| {
            LauncherError::file_unavailable(format!(
                "Utilities data could not be persisted; current entries were preserved: {error}"
            ))
        })?;
        self.disk_snapshot = Some(contents.into_bytes());
        Ok(())
    }
}

pub(super) fn validate_stored_entries(entries: &[Entry]) -> Result<(), LauncherError> {
    if entries.len() > entry::MAX_ENTRIES {
        return Err(LauncherError::invalid_request(format!(
            "Entry count exceeds the {} item limit",
            entry::MAX_ENTRIES
        )));
    }
    for (index, item) in entries.iter().enumerate() {
        entry::validate_entry(item)?;
        if entries[..index].iter().any(|earlier| earlier.id == item.id) {
            return Err(LauncherError::invalid_request("Utility IDs must be unique"));
        }
    }
    Ok(())
}

fn parse_utilities(contents: &[u8]) -> Result<Vec<Entry>, LauncherError> {
    let text = std::str::from_utf8(contents)
        .map_err(|_| LauncherError::file_unavailable("Utilities data is not UTF-8"))?;
    let persisted: LoadedUtilities = toml::from_str(text).map_err(|error: toml::de::Error| {
        LauncherError::file_unavailable(format!("Invalid utilities TOML: {}", error.message()))
    })?;
    if persisted.schema_version != SCHEMA_VERSION {
        return Err(LauncherError::file_unavailable(format!(
            "Unsupported utilities schema version {}",
            persisted.schema_version
        )));
    }
    let mut entries = persisted.entries;
    validate_stored_entries(&entries)?;
    for item in &mut entries {
        item.rebuild_search_cache();
    }
    Ok(entries)
}

fn read_utilities_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "utilities data path is not a regular file",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "utilities data file exceeds the supported size",
        ));
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut contents)?;
    if contents.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "utilities data file exceeds the supported size",
        ));
    }
    Ok(contents)
}

fn atomic_replace(path: &Path, contents: &[u8], expected: Option<&[u8]>) -> io::Result<()> {
    let directory = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "utilities path has no parent directory",
        )
    })?;
    fs::create_dir_all(directory)?;
    ensure_expected_target(path, expected)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "utilities path has no valid filename",
            )
        })?;
    let (temporary, mut file) = create_temporary_file(directory, file_name)?;
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);

    if let Err(error) = ensure_expected_target(path, expected) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

fn ensure_expected_target(path: &Path, expected: Option<&[u8]>) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "utilities data path is not a regular file",
                ));
            }
            let Some(expected) = expected else {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "refusing to replace an unexpected utilities data file",
                ));
            };
            let actual = read_utilities_file(path)?;
            if actual != expected {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "utilities data changed since it was loaded",
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && expected.is_none() => Ok(()),
        Err(error) => Err(error),
    }
}

fn create_temporary_file(directory: &Path, file_name: &str) -> io::Result<(PathBuf, fs::File)> {
    for _ in 0..16 {
        let number = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{file_name}.tmp-{}-{number}", std::process::id()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique utilities temporary file",
    ))
}
