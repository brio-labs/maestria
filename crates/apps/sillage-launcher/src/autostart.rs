use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

use crate::LauncherError;

const FILE_NAME: &str = "io.github.briolabs.Sillage.Launcher.desktop";
const CONTENT: &str = concat!(
    "[Desktop Entry]\n",
    "Type=Application\n",
    "Name=Sillage Launcher\n",
    "Exec=sillage-launcher --background\n",
    "Terminal=false\n",
    "X-Sillage-Autostart-Owner=launcher-v1\n",
);

fn path(config: Result<PathBuf, String>) -> Result<PathBuf, LauncherError> {
    let config = config.map_err(LauncherError::settings_failed)?;
    let base = config
        .parent()
        .ok_or_else(|| LauncherError::settings_failed("The autostart directory is unavailable"))?;
    Ok(base.join("autostart").join(FILE_NAME))
}

pub(crate) fn enabled(config: Result<PathBuf, String>) -> Result<bool, LauncherError> {
    enabled_at(&path(config)?)
}

fn enabled_at(path: &Path) -> Result<bool, LauncherError> {
    Ok(owned_entry(path)?.is_some())
}

pub(crate) fn set_enabled(
    config: Result<PathBuf, String>,
    enabled: bool,
) -> Result<(), LauncherError> {
    set_enabled_at(&path(config)?, enabled)
}

fn set_enabled_at(path: &Path, enabled: bool) -> Result<(), LauncherError> {
    let existing = owned_entry(path)?;
    if enabled {
        if existing.is_some() {
            return Ok(());
        }
        let parent = path.parent().ok_or_else(|| {
            LauncherError::settings_failed("The autostart directory is unavailable")
        })?;
        create_directories(parent)?;
        create_entry(path)
    } else if let Some(identity) = existing {
        if remove_owned_entry(path, &identity)? {
            Ok(())
        } else {
            Err(LauncherError::settings_failed(
                "The autostart entry changed and was preserved",
            ))
        }
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct EntryIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(not(unix))]
    length: u64,
}

impl EntryIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            Self {
                dev: metadata.dev(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                length: metadata.len(),
            }
        }
    }

    fn matches(self, metadata: &fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            self.dev == metadata.dev() && self.ino == metadata.ino()
        }
        #[cfg(not(unix))]
        {
            self.length == metadata.len()
        }
    }
}

fn parent_directories_exist(path: &Path) -> Result<bool, LauncherError> {
    let Some(parent) = path.parent() else {
        return Err(LauncherError::settings_failed(
            "The autostart directory is unavailable",
        ));
    };
    let mut current = PathBuf::new();
    for component in parent.components() {
        current.push(component.as_os_str());
        if current.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => validate_directory(&metadata)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(settings_error(error)),
        }
    }
    Ok(true)
}

fn owned_entry(path: &Path) -> Result<Option<EntryIdentity>, LauncherError> {
    if !parent_directories_exist(path)? {
        return Ok(None);
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(settings_error(error)),
    };
    validate_entry_metadata(&metadata)?;
    if metadata.len() > CONTENT.len() as u64 {
        return Err(foreign_entry());
    }

    let identity = EntryIdentity::from_metadata(&metadata);
    let mut file = open_readonly(path).map_err(settings_error)?;
    let opened_metadata = file.metadata().map_err(settings_error)?;
    validate_entry_metadata(&opened_metadata)?;
    if !identity.matches(&opened_metadata) {
        return Err(changed_entry());
    }

    let mut contents = Vec::with_capacity(CONTENT.len() + 1);
    Read::by_ref(&mut file)
        .take((CONTENT.len() + 1) as u64)
        .read_to_end(&mut contents)
        .map_err(settings_error)?;
    if contents != CONTENT.as_bytes() {
        return Err(foreign_entry());
    }
    Ok(Some(identity))
}

fn validate_entry_metadata(metadata: &fs::Metadata) -> Result<(), LauncherError> {
    if !metadata.file_type().is_file() {
        return Err(foreign_entry());
    }
    #[cfg(target_os = "linux")]
    {
        let uid = unsafe { libc::geteuid() };
        if metadata.uid() != uid || metadata.permissions().mode() & 0o077 != 0 {
            return Err(foreign_entry());
        }
    }
    Ok(())
}

fn open_readonly(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    options.open(path)
}

fn remove_owned_entry(path: &Path, expected: &EntryIdentity) -> Result<bool, LauncherError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(settings_error(error)),
    };
    if !expected.matches(&metadata) {
        return Ok(false);
    }
    validate_entry_metadata(&metadata)?;
    let Some(current) = owned_entry(path)? else {
        return Ok(false);
    };
    if current.dev_ino_differs(expected) {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path).map_err(settings_error)?;
    validate_entry_metadata(&metadata)?;
    if !expected.matches(&metadata) {
        return Ok(false);
    }
    fs::remove_file(path).map_err(settings_error)?;
    Ok(true)
}
impl EntryIdentity {
    fn dev_ino_differs(self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.dev != other.dev || self.ino != other.ino
        }
        #[cfg(not(unix))]
        {
            self.length != other.length
        }
    }
}

struct TemporaryEntry {
    path: PathBuf,
    identity: EntryIdentity,
}

impl Drop for TemporaryEntry {
    fn drop(&mut self) {
        let _ = remove_temporary_if_same(&self.path, self.identity);
    }
}

fn create_entry(path: &Path) -> Result<(), LauncherError> {
    if !parent_directories_exist(path)? {
        return Err(changed_entry());
    }
    let parent = path
        .parent()
        .ok_or_else(|| LauncherError::settings_failed("The autostart directory is unavailable"))?;
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".sillage-autostart-{}-{nonce}.tmp",
        std::process::id()
    ));

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(target_os = "linux")]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);

    let mut file = options.open(&temporary).map_err(settings_error)?;
    let metadata = file.metadata().map_err(settings_error)?;
    validate_entry_metadata(&metadata)?;
    let owned_temporary = TemporaryEntry {
        path: temporary.clone(),
        identity: EntryIdentity::from_metadata(&metadata),
    };

    file.write_all(CONTENT.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(settings_error)?;
    let current = fs::symlink_metadata(&temporary).map_err(settings_error)?;
    validate_entry_metadata(&current)?;
    if !owned_temporary.identity.matches(&current) {
        return Err(changed_entry());
    }

    fs::hard_link(&temporary, path).map_err(settings_error)?;
    let Some(installed) = owned_entry(path)? else {
        return Err(changed_entry());
    };
    if installed.dev_ino_differs(&owned_temporary.identity) {
        return Err(changed_entry());
    }
    drop(owned_temporary);
    Ok(())
}

fn remove_temporary_if_same(path: &Path, identity: EntryIdentity) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || !identity.matches(&metadata) {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Ok(());
    }
    fs::remove_file(path)
}

fn create_directories(path: &Path) -> Result<(), LauncherError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) => current.push(component.as_os_str()),
            Component::RootDir | Component::CurDir | Component::ParentDir => {
                current.push(component.as_os_str());
            }
            Component::Normal(name) => current.push(name),
        }
        if current.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => validate_directory(&metadata)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                create_private_directory(&current)?;
            }
            Err(error) => return Err(settings_error(error)),
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), LauncherError> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => {
            let metadata = fs::symlink_metadata(path).map_err(settings_error)?;
            validate_directory(&metadata)?;
            if !is_usable_private_directory(&metadata) {
                return Err(changed_entry());
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(settings_error)?;
            validate_directory(&metadata)?;
            if !is_usable_private_directory(&metadata) {
                return Err(changed_entry());
            }
            Ok(())
        }
        Err(error) => Err(settings_error(error)),
    }
}

fn validate_directory(metadata: &fs::Metadata) -> Result<(), LauncherError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::settings_failed(
            "The autostart path contains a non-directory or symbolic link",
        ));
    }
    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode();
        if mode & 0o022 != 0 && mode & 0o1000 == 0 {
            return Err(LauncherError::settings_failed(
                "The autostart path contains a directory writable by other users",
            ));
        }
    }
    Ok(())
}

fn owned_by_current_user(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "linux")]
    {
        metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = metadata;
        true
    }
}

fn is_private_directory(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}
fn is_usable_private_directory(metadata: &fs::Metadata) -> bool {
    if !owned_by_current_user(metadata) || !is_private_directory(metadata) {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o700 == 0o700
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn foreign_entry() -> LauncherError {
    LauncherError::settings_failed(
        "An existing autostart entry was preserved because it is not an owned launcher entry",
    )
}

fn changed_entry() -> LauncherError {
    LauncherError::settings_failed("The autostart path changed and was preserved")
}

fn settings_error(error: io::Error) -> LauncherError {
    LauncherError::settings_failed(error.to_string())
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests;
