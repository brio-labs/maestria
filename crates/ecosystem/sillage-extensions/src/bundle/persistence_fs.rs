use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::BundleError;
pub(in crate::bundle) fn store_directory(root: &Path, name: &str) -> Result<PathBuf, BundleError> {
    let path = root.join(name);
    checked_directory(&path, root)?.ok_or(BundleError::InvalidState(
        "extension store directory is missing",
    ))?;
    Ok(path)
}

pub(in crate::bundle) fn checked_directory(
    path: &Path,
    parent: &Path,
) -> Result<Option<PathBuf>, BundleError> {
    let canonical = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
    let canonical_parent = fs::canonicalize(parent).map_err(|source| io_error(parent, source))?;
    if !canonical.starts_with(&canonical_parent) {
        return Err(BundleError::InvalidState(
            "extension store directory escapes its parent",
        ));
    }
    Ok(Some(canonical))
}

pub(in crate::bundle) fn remove_sealed_package(path: &Path) -> Result<(), BundleError> {
    remove_tree(path)
}

pub(in crate::bundle) fn remove_tree(path: &Path) -> Result<(), BundleError> {
    #[cfg(target_os = "linux")]
    {
        secure_remove::remove_tree(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(io_error(
            path,
            std::io::Error::new(
                ErrorKind::Unsupported,
                "secure recursive extension removal requires Linux descriptor-relative filesystem operations",
            ),
        ))
    }
}

#[cfg(target_os = "linux")]
mod secure_remove {
    use std::ffi::{CStr, CString};
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use rustix::fs::{AtFlags, Mode, OFlags, RawDir, fchmod, open, openat, unlinkat};
    use rustix::io::Errno;

    use super::{BundleError, io_error};

    pub(super) fn remove_tree(path: &Path) -> Result<(), BundleError> {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let name = match path.file_name() {
            Some(name) => name,
            None => {
                return Err(io_error(
                    path,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "extension removal path has no final component",
                    ),
                ));
            }
        };
        let parent_directory = match open(
            parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(Errno::NOENT) => return Ok(()),
            Err(source) => return Err(io_error(parent, source.into())),
        };
        let name = CString::new(name.as_bytes()).map_err(|source| {
            io_error(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, source),
            )
        })?;
        remove_entry_at(&parent_directory, &name, path)
    }

    fn remove_entry_at(
        parent: &rustix::fd::OwnedFd,
        name: &CStr,
        path: &Path,
    ) -> Result<(), BundleError> {
        let directory = match openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(Errno::NOENT) => return Ok(()),
            Err(source) if source == Errno::NOTDIR || source == Errno::LOOP => {
                return unlink_entry_at(parent, name, path);
            }
            Err(source) => return Err(io_error(path, source.into())),
        };

        fchmod(&directory, Mode::from_raw_mode(0o700))
            .map_err(|source| io_error(path, source.into()))?;

        let mut buffer = [MaybeUninit::<u8>::uninit(); 8192];
        let mut entries = RawDir::new(&directory, &mut buffer);
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|source| io_error(path, source.into()))?;
            let entry_name = entry.file_name();
            if entry_name.to_bytes() == b"." || entry_name.to_bytes() == b".." {
                continue;
            }
            let child_path = path.join(std::ffi::OsStr::from_bytes(entry_name.to_bytes()));
            remove_entry_at(&directory, entry_name, &child_path)?;
        }

        match unlinkat(parent, name, AtFlags::REMOVEDIR) {
            Ok(()) | Err(Errno::NOENT) => Ok(()),
            Err(source) if source == Errno::NOTDIR || source == Errno::LOOP => {
                unlink_entry_at(parent, name, path)
            }
            Err(source) => Err(io_error(path, source.into())),
        }
    }

    fn unlink_entry_at(
        parent: &rustix::fd::OwnedFd,
        name: &CStr,
        path: &Path,
    ) -> Result<(), BundleError> {
        match unlinkat(parent, name, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => Ok(()),
            Err(source) => Err(io_error(path, source.into())),
        }
    }
}

pub(in crate::bundle) fn ensure_directory(path: &Path, mode: u32) -> Result<(), BundleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BundleError::InvalidState(
                "extension store contains a non-directory path component",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| io_error(path, source))?;
        }
        Err(source) => return Err(io_error(path, source)),
    }
    set_directory_permissions(path, mode)
}

pub(in crate::bundle) fn set_directory_permissions(
    path: &Path,
    mode: u32,
) -> Result<(), BundleError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|source| io_error(path, source))
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        Err(io_error(
            path,
            std::io::Error::new(
                ErrorKind::Unsupported,
                "extension bundle storage requires Unix permission bits",
            ),
        ))
    }
}

pub(in crate::bundle) fn set_file_permissions(
    path: &Path,
    mode: u32,
) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        Err(std::io::Error::new(
            ErrorKind::Unsupported,
            "extension bundle storage requires Unix permission bits",
        ))
    }
}

pub(in crate::bundle) fn sync_directory(path: &Path) -> Result<(), BundleError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

pub(in crate::bundle) fn io_error(path: &Path, source: std::io::Error) -> BundleError {
    BundleError::Io {
        path: path.to_path_buf(),
        source,
    }
}
