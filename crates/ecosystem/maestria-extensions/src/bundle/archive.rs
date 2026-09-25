use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use zip::ZipArchive;

use crate::bundle::source::{self, PackageFiles};
use crate::bundle::{BundleError, MAX_PACKAGE_BYTES, MAX_PACKAGE_FILES};

const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;

pub(super) fn read_archive(path: &Path) -> Result<PackageFiles, BundleError> {
    let mut file = source::open_regular_file(path)?;
    let metadata = file.metadata().map_err(|source| BundleError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(BundleError::InvalidPackage(
            "compressed archive exceeds the size limit".to_owned(),
        ));
    }
    validate_bounded_directory(&mut file, metadata.len(), path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(BundleError::InvalidPackage(
            "archive contains too many entries".to_owned(),
        ));
    }

    let mut files = PackageFiles::new();
    let mut seen_paths = BTreeSet::new();
    let mut total_bytes = 0u64;
    for index in 0..archive.len() {
        let (raw_name, is_directory, declared_size, mode) = {
            let entry = archive.by_index(index)?;
            (
                entry.name().to_owned(),
                entry.is_dir(),
                entry.size(),
                entry.unix_mode(),
            )
        };
        let (relative_path, is_directory) = normalize_entry_name(&raw_name, is_directory)?;
        if let Some(mode) = mode {
            let file_type = mode & 0o170000;
            let expected_type = if is_directory { 0o040000 } else { 0o100000 };
            if file_type != 0 && file_type != expected_type {
                return Err(BundleError::InvalidPackage(
                    "archive contains a symbolic link or special filesystem entry".to_owned(),
                ));
            }
        }
        if !seen_paths.insert(relative_path.clone()) {
            return Err(BundleError::InvalidPackage(
                "archive contains duplicate paths".to_owned(),
            ));
        }
        if is_directory {
            if declared_size != 0 {
                return Err(BundleError::InvalidPackage(
                    "archive directory entry contains file data".to_owned(),
                ));
            }
            continue;
        }
        if files.len() >= MAX_PACKAGE_FILES {
            return Err(BundleError::InvalidPackage(
                "archive contains too many files".to_owned(),
            ));
        }
        if declared_size > MAX_PACKAGE_BYTES.saturating_sub(total_bytes) {
            return Err(BundleError::InvalidPackage(
                "archive exceeds the uncompressed size limit".to_owned(),
            ));
        }
        let mut entry = archive.by_index(index)?;
        let mut bytes = Vec::new();
        let mut limited = (&mut entry).take(MAX_PACKAGE_BYTES - total_bytes + 1);
        limited
            .read_to_end(&mut bytes)
            .map_err(|source| BundleError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        let actual_size = bytes.len() as u64;
        if actual_size > MAX_PACKAGE_BYTES - total_bytes || actual_size != declared_size {
            return Err(BundleError::InvalidPackage(
                "archive entry size differs from its declared bounded size".to_owned(),
            ));
        }
        total_bytes += actual_size;
        if has_file_ancestor(&relative_path, &files) || has_file_descendant(&relative_path, &files)
        {
            return Err(BundleError::InvalidPackage(
                "archive file path conflicts with another file path".to_owned(),
            ));
        }
        files.insert(relative_path, bytes);
    }
    Ok(files)
}
const EOCD_MIN_SIZE: usize = 22;
const ZIP_TRAILER_MAX: u64 = (EOCD_MIN_SIZE + u16::MAX as usize) as u64;
const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";

fn validate_bounded_directory(
    file: &mut std::fs::File,
    file_size: u64,
    path: &Path,
) -> Result<(), BundleError> {
    if file_size < EOCD_MIN_SIZE as u64 {
        return Err(BundleError::InvalidPackage(
            "archive is too short to contain a ZIP directory".to_owned(),
        ));
    }
    let tail_size = file_size.min(ZIP_TRAILER_MAX);
    let tail_start = file_size - tail_size;
    let tail_length = usize::try_from(tail_size).map_err(|_| {
        BundleError::InvalidPackage("archive trailer size is not representable".to_owned())
    })?;
    file.seek(SeekFrom::Start(tail_start))
        .map_err(|source| BundleError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut tail = Vec::with_capacity(tail_length);
    (&mut *file)
        .take(tail_size)
        .read_to_end(&mut tail)
        .map_err(|source| BundleError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if tail.len() != tail_length {
        return Err(BundleError::InvalidPackage(
            "archive trailer is truncated".to_owned(),
        ));
    }
    let eocd_index = find_end_of_directory(&tail)?;
    if has_zip64_locator(&tail, eocd_index) {
        return Err(BundleError::InvalidPackage(
            "ZIP64 archives are not supported by the bounded bundle format".to_owned(),
        ));
    }
    let disk = read_u16(&tail, eocd_index + 4);
    let central_disk = read_u16(&tail, eocd_index + 6);
    let disk_entries = read_u16(&tail, eocd_index + 8);
    let total_entries = read_u16(&tail, eocd_index + 10);
    let central_size = read_u32(&tail, eocd_index + 12);
    let central_offset = read_u32(&tail, eocd_index + 16);
    let (
        Some(disk),
        Some(central_disk),
        Some(disk_entries),
        Some(total_entries),
        Some(central_size),
        Some(central_offset),
    ) = (
        disk,
        central_disk,
        disk_entries,
        total_entries,
        central_size,
        central_offset,
    )
    else {
        return Err(BundleError::InvalidPackage(
            "archive directory header is truncated".to_owned(),
        ));
    };
    if disk != 0 || central_disk != 0 || disk_entries != total_entries {
        return Err(BundleError::InvalidPackage(
            "multi-disk ZIP archives are not supported".to_owned(),
        ));
    }
    if total_entries == u16::MAX
        || central_size == u32::MAX
        || central_offset == u32::MAX
        || usize::from(total_entries) > MAX_ARCHIVE_ENTRIES
    {
        return Err(BundleError::InvalidPackage(
            "archive directory exceeds the bounded entry limit".to_owned(),
        ));
    }
    let central_end = u64::from(central_offset)
        .checked_add(u64::from(central_size))
        .ok_or_else(|| {
            BundleError::InvalidPackage("archive directory offset overflows".to_owned())
        })?;
    let eocd_absolute = tail_start.checked_add(eocd_index as u64).ok_or_else(|| {
        BundleError::InvalidPackage("archive trailer offset overflows".to_owned())
    })?;
    if central_end > eocd_absolute {
        return Err(BundleError::InvalidPackage(
            "archive directory extends beyond its end record".to_owned(),
        ));
    }
    Ok(())
}

fn find_end_of_directory(tail: &[u8]) -> Result<usize, BundleError> {
    let last_candidate = tail
        .len()
        .checked_sub(EOCD_MIN_SIZE)
        .ok_or_else(|| BundleError::InvalidPackage("archive trailer is too short".to_owned()))?;
    for index in (0..=last_candidate).rev() {
        if tail.get(index..index + EOCD_SIGNATURE.len()) != Some(EOCD_SIGNATURE.as_slice()) {
            continue;
        }
        let Some(comment_length) = read_u16(tail, index + 20) else {
            continue;
        };
        let Some(end) = index
            .checked_add(EOCD_MIN_SIZE)
            .and_then(|value| value.checked_add(usize::from(comment_length)))
        else {
            continue;
        };
        if end == tail.len() {
            return Ok(index);
        }
    }
    Err(BundleError::InvalidPackage(
        "archive has no valid ZIP end record".to_owned(),
    ))
}

fn has_zip64_locator(tail: &[u8], eocd_index: usize) -> bool {
    let Some(locator_index) = eocd_index.checked_sub(20) else {
        return false;
    };
    tail.get(locator_index..locator_index + ZIP64_LOCATOR_SIGNATURE.len())
        == Some(ZIP64_LOCATOR_SIGNATURE.as_slice())
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let bytes = bytes.get(offset..end)?;
    let bytes: [u8; 2] = bytes.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes = bytes.get(offset..end)?;
    let bytes: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn normalize_entry_name(
    raw_name: &str,
    directory_hint: bool,
) -> Result<(String, bool), BundleError> {
    if raw_name.is_empty()
        || raw_name.contains(['\\', ':', '%', '\0'])
        || raw_name.starts_with('/')
        || raw_name.contains("//")
    {
        return Err(BundleError::InvalidPackage(
            "archive path is empty, absolute, or contains a forbidden character".to_owned(),
        ));
    }
    let directory = directory_hint || raw_name.ends_with('/');
    let path = match raw_name.strip_suffix('/') {
        Some(trimmed) if raw_name.ends_with('/') => trimmed,
        _ => raw_name,
    };
    if path.ends_with('/') {
        return Err(BundleError::InvalidPackage(
            "archive path contains an empty component".to_owned(),
        ));
    }
    source::validate_relative_path(path)?;
    Ok((path.to_owned(), directory))
}

fn has_file_ancestor(path: &str, files: &PackageFiles) -> bool {
    let mut ancestor = path;
    while let Some((parent, _)) = ancestor.rsplit_once('/') {
        if files.contains_key(parent) {
            return true;
        }
        ancestor = parent;
    }
    false
}

fn has_file_descendant(path: &str, files: &PackageFiles) -> bool {
    let prefix = format!("{path}/");
    files.keys().any(|existing| existing.starts_with(&prefix))
}
