use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::bundle::{BundleError, MAX_PACKAGE_BYTES, MAX_PACKAGE_FILES};

pub(super) type PackageFiles = BTreeMap<String, Vec<u8>>;

const MAX_DIRECTORY_ENTRIES: usize = 256;
const MAX_DIRECTORY_DEPTH: usize = 16;

pub(super) fn read_directory(root: &Path) -> Result<PackageFiles, BundleError> {
    let root_metadata = fs::symlink_metadata(root).map_err(|source| BundleError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(BundleError::InvalidPackage(
            "package source must be a real directory".to_owned(),
        ));
    }
    let canonical_root = fs::canonicalize(root).map_err(|source| BundleError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut pending = vec![(canonical_root.clone(), String::new(), 0usize)];
    let mut files = PackageFiles::new();
    let mut entries_seen = 0usize;
    let mut total_bytes = 0u64;

    while let Some((directory, relative_directory, depth)) = pending.pop() {
        if depth > MAX_DIRECTORY_DEPTH {
            return Err(BundleError::InvalidPackage(
                "package directory nesting exceeds the maximum depth".to_owned(),
            ));
        }
        let mut children = read_directory_children(&directory, &mut entries_seen)?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            read_directory_child(
                &canonical_root,
                &relative_directory,
                depth,
                child,
                &mut pending,
                &mut files,
                &mut total_bytes,
            )?;
        }
    }
    Ok(files)
}

fn read_directory_children(
    directory: &Path,
    entries_seen: &mut usize,
) -> Result<Vec<fs::DirEntry>, BundleError> {
    let iterator = fs::read_dir(directory).map_err(|source| BundleError::Io {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut children = Vec::new();
    for child in iterator {
        let child = child.map_err(|source| BundleError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        *entries_seen = entries_seen.saturating_add(1);
        if *entries_seen > MAX_DIRECTORY_ENTRIES {
            return Err(BundleError::InvalidPackage(
                "package contains too many filesystem entries".to_owned(),
            ));
        }
        children.push(child);
    }
    Ok(children)
}

fn read_directory_child(
    canonical_root: &Path,
    relative_directory: &str,
    depth: usize,
    child: fs::DirEntry,
    pending: &mut Vec<(PathBuf, String, usize)>,
    files: &mut PackageFiles,
    total_bytes: &mut u64,
) -> Result<(), BundleError> {
    let file_name = child.file_name();
    let Some(file_name) = file_name.to_str() else {
        return Err(BundleError::InvalidPackage(
            "package paths must be valid UTF-8".to_owned(),
        ));
    };
    validate_segment(file_name)?;
    let relative_path = if relative_directory.is_empty() {
        file_name.to_owned()
    } else {
        format!("{relative_directory}/{file_name}")
    };
    validate_relative_path(&relative_path)?;
    let path = child.path();
    let metadata = fs::symlink_metadata(&path).map_err(|source| BundleError::Io {
        path: path.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(BundleError::InvalidPackage(
            "package contains a symbolic link".to_owned(),
        ));
    }
    let canonical_path = fs::canonicalize(&path).map_err(|source| BundleError::Io {
        path: path.clone(),
        source,
    })?;
    if !canonical_path.starts_with(canonical_root) {
        return Err(BundleError::InvalidPackage(
            "package path escapes its source directory".to_owned(),
        ));
    }
    if metadata.is_dir() {
        if depth == MAX_DIRECTORY_DEPTH {
            return Err(BundleError::InvalidPackage(
                "package directory nesting exceeds the maximum depth".to_owned(),
            ));
        }
        pending.push((canonical_path, relative_path, depth + 1));
    } else if metadata.is_file() {
        if files.len() >= MAX_PACKAGE_FILES {
            return Err(BundleError::InvalidPackage(
                "package contains too many files".to_owned(),
            ));
        }
        if metadata.len() > MAX_PACKAGE_BYTES.saturating_sub(*total_bytes) {
            return Err(BundleError::InvalidPackage(
                "package exceeds the uncompressed size limit".to_owned(),
            ));
        }
        let bytes = read_file_bounded(&path, MAX_PACKAGE_BYTES.saturating_sub(*total_bytes))?;
        *total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| BundleError::InvalidPackage("package size overflow".to_owned()))?;
        if files.insert(relative_path, bytes).is_some() {
            return Err(BundleError::InvalidPackage(
                "package contains duplicate file paths".to_owned(),
            ));
        }
    } else {
        return Err(BundleError::InvalidPackage(
            "package contains a non-regular filesystem entry".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_relative_path(path: &str) -> Result<(), BundleError> {
    if path.is_empty() || path.len() > 240 || path.contains(['\\', ':', '%']) {
        return Err(BundleError::InvalidPackage(
            "package path is empty, too long, or contains a forbidden character".to_owned(),
        ));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > MAX_DIRECTORY_DEPTH {
        return Err(BundleError::InvalidPackage(
            "package path exceeds the maximum nesting depth".to_owned(),
        ));
    }
    for segment in segments {
        validate_segment(segment)?;
    }
    Ok(())
}

fn validate_segment(segment: &str) -> Result<(), BundleError> {
    let Some(first) = segment.as_bytes().first() else {
        return Err(BundleError::InvalidPackage(
            "package path contains an empty component".to_owned(),
        ));
    };
    if !first.is_ascii_alphanumeric() && *first != b'_' && *first != b'-' {
        return Err(BundleError::InvalidPackage(
            "package path component has an invalid first character".to_owned(),
        ));
    }
    if segment
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
        || matches!(segment, "." | "..")
    {
        return Err(BundleError::InvalidPackage(
            "package path component contains a forbidden character".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn open_regular_file(path: &Path) -> Result<File, BundleError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| BundleError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BundleError::InvalidPackage(
            "package input must be a regular non-symbolic-link file".to_owned(),
        ));
    }
    #[cfg(not(target_os = "linux"))]
    return Err(BundleError::InvalidState(
        "safe package input requires Linux O_NOFOLLOW",
    ));
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NOFOLLOW: i32 = 0o400000;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW)
            .open(path)
            .map_err(|source| BundleError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if !file
            .metadata()
            .map_err(|source| BundleError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .is_file()
        {
            return Err(BundleError::InvalidPackage(
                "package input changed type while being opened".to_owned(),
            ));
        }
        Ok(file)
    }
}

fn read_file_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, BundleError> {
    let mut file = open_regular_file(path)?;
    let mut bytes = Vec::new();
    let mut limited = (&mut file).take(limit.saturating_add(1));
    limited
        .read_to_end(&mut bytes)
        .map_err(|source| BundleError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(BundleError::InvalidPackage(
            "package exceeds the uncompressed size limit".to_owned(),
        ));
    }
    let after_read = file.metadata().map_err(|source| BundleError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if after_read.len() != bytes.len() as u64 {
        return Err(BundleError::InvalidPackage(
            "package file changed while being read".to_owned(),
        ));
    }
    Ok(bytes)
}
