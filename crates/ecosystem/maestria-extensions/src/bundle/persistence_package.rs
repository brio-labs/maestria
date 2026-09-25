use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::bundle::source::{self, PackageFiles};
use crate::bundle::{BundleError, MAX_PACKAGE_BYTES, PackageIdentity};
use sha2::{Digest, Sha256};

use super::{
    InstallationRecord, STAGING_ATTEMPTS, StoreState, TEMPORARY_ID, checked_directory,
    ensure_directory, io_error, remove_tree, store_directory, valid_extension_id,
    validate_package_identity,
};

const MAX_PACKAGE_STORE_ENTRIES: usize = 4096;

pub(in crate::bundle) fn digest_files(files: &PackageFiles) -> String {
    let mut digest = Sha256::new();
    digest.update(b"maestria-extension-package-v1\0");
    for (path, bytes) in files {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    let bytes = digest.finalize();
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(hex_digit(byte >> 4));
        result.push(hex_digit(byte & 0x0f));
    }
    result
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0 => '0',
        1 => '1',
        2 => '2',
        3 => '3',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => '7',
        8 => '8',
        9 => '9',
        10 => 'a',
        11 => 'b',
        12 => 'c',
        13 => 'd',
        14 => 'e',
        _ => 'f',
    }
}

pub(in crate::bundle) fn seal_package(
    root: &Path,
    id: &str,
    files: &PackageFiles,
    identity: &PackageIdentity,
) -> Result<(PathBuf, bool), BundleError> {
    if !valid_extension_id(id) || digest_files(files) != identity.sha256 {
        return Err(BundleError::InvalidPackage(
            "package identity does not match the validated source files".to_owned(),
        ));
    }
    let packages_root = store_directory(root, "packages")?;
    let package_parent = packages_root.join(id);
    ensure_directory(&package_parent, 0o700)?;
    let final_path = package_parent.join(package_directory_name(identity));
    if let Some(existing) = existing_directory(&final_path)? {
        let existing_files = source::read_directory(&existing)?;
        if existing_files != *files {
            return Err(BundleError::IdentityConflict);
        }
        return Ok((existing, false));
    }
    let staging_root = store_directory(root, "staging")?;
    let staging_path = create_staging_directory(&staging_root)?;
    let mut renamed = false;
    let stage_result = populate_stage(&staging_path, files)
        .and_then(|()| {
            fs::rename(&staging_path, &final_path)
                .map_err(|source| io_error(&final_path, source))?;
            renamed = true;
            Ok(())
        })
        .and_then(|()| super::set_directory_permissions(&final_path, 0o555))
        .and_then(|()| super::sync_directory(&package_parent));
    match stage_result {
        Ok(()) => Ok((final_path, true)),
        Err(install) => {
            let cleanup_path = if renamed { &final_path } else { &staging_path };
            match super::remove_sealed_package(cleanup_path) {
                Ok(()) => Err(install),
                Err(cleanup) => Err(BundleError::InstallCleanup {
                    install: Box::new(install),
                    cleanup: Box::new(cleanup),
                }),
            }
        }
    }
}

fn create_staging_directory(root: &Path) -> Result<PathBuf, BundleError> {
    for _ in 0..STAGING_ATTEMPTS {
        let sequence = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("package-{}-{sequence}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&path, source)),
        }
    }
    Err(BundleError::InvalidState(
        "could not allocate a unique staging directory",
    ))
}

fn populate_stage(path: &Path, files: &PackageFiles) -> Result<(), BundleError> {
    let mut total_bytes = 0u64;
    for (relative_path, bytes) in files {
        source::validate_relative_path(relative_path)?;
        total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| BundleError::InvalidPackage("package size overflow".to_owned()))?;
        if total_bytes > MAX_PACKAGE_BYTES {
            return Err(BundleError::InvalidPackage(
                "package exceeds the uncompressed size limit".to_owned(),
            ));
        }
        write_staged_file(path, relative_path, bytes)?;
    }
    seal_directory_tree(path)
}

fn write_staged_file(root: &Path, relative_path: &str, bytes: &[u8]) -> Result<(), BundleError> {
    let mut current = root.to_path_buf();
    let mut segments = relative_path.split('/').peekable();
    while let Some(segment) = segments.next() {
        current.push(segment);
        if segments.peek().is_some() {
            match fs::create_dir(&current) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(&current)
                        .map_err(|source| io_error(&current, source))?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(BundleError::InvalidPackage(
                            "package file paths contain a directory conflict".to_owned(),
                        ));
                    }
                }
                Err(source) => return Err(io_error(&current, source)),
            }
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&current)
                .map_err(|source| io_error(&current, source))?;
            file.write_all(bytes)
                .map_err(|source| io_error(&current, source))?;
            super::set_file_permissions(&current, 0o444)
                .map_err(|source| io_error(&current, source))?;
            file.sync_all()
                .map_err(|source| io_error(&current, source))?;
        }
    }
    Ok(())
}

fn seal_directory_tree(path: &Path) -> Result<(), BundleError> {
    let entries = fs::read_dir(path).map_err(|source| io_error(path, source))?;
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| io_error(path, source))?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|source| io_error(&child, source))?;
        if metadata.file_type().is_symlink() {
            return Err(BundleError::InvalidPackage(
                "staging tree unexpectedly contains a symbolic link".to_owned(),
            ));
        }
        if metadata.is_dir() {
            seal_directory_tree(&child)?;
            directories.push(child);
        }
    }
    for directory in directories {
        super::sync_directory(&directory)?;
        super::set_directory_permissions(&directory, 0o555)?;
    }
    super::sync_directory(path)?;
    Ok(())
}

fn existing_directory(path: &Path) -> Result<Option<PathBuf>, BundleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(BundleError::IdentityConflict)
        }
        Ok(_) => Ok(Some(path.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, source)),
    }
}

pub(in crate::bundle) fn package_directory(
    root: &Path,
    installation: &InstallationRecord,
) -> Result<PathBuf, BundleError> {
    validate_package_identity(&installation.package)?;
    if !valid_extension_id(&installation.extension_id)
        || installation.grants_for != installation.package
    {
        return Err(BundleError::InvalidState(
            "installation package identity is invalid",
        ));
    }
    let packages = store_directory(root, "packages")?;
    let extension_path = packages.join(&installation.extension_id);
    let directory_name = package_directory_name(&installation.package);
    let Some(package_parent) = checked_directory(&extension_path, &packages)? else {
        return Ok(extension_path.join(directory_name));
    };
    Ok(package_parent.join(directory_name))
}

fn package_directory_name(identity: &PackageIdentity) -> String {
    format!("{}-{}", identity.version, identity.sha256)
}

pub(in crate::bundle) fn prune_unreferenced_packages(
    root: &Path,
    state: &StoreState,
) -> Result<(), BundleError> {
    let packages = store_directory(root, "packages")?;
    let mut entries_seen = 0usize;
    for entry in fs::read_dir(&packages).map_err(|source| io_error(&packages, source))? {
        let entry = entry.map_err(|source| io_error(&packages, source))?;
        count_store_entry(&mut entries_seen)?;
        let extension_path = entry.path();
        let entry_name = entry.file_name();
        let tracked = entry_name.to_str().and_then(|id| {
            state
                .installations
                .iter()
                .find(|installation| installation.extension_id == id)
        });
        let Some(installation) = tracked else {
            remove_tree(&extension_path)?;
            continue;
        };
        let metadata = fs::symlink_metadata(&extension_path)
            .map_err(|source| io_error(&extension_path, source))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let active_package = package_directory(root, installation)?;
        for version in
            fs::read_dir(&extension_path).map_err(|source| io_error(&extension_path, source))?
        {
            let version = version.map_err(|source| io_error(&extension_path, source))?;
            count_store_entry(&mut entries_seen)?;
            let version_path = version.path();
            if version_path.file_name() != active_package.file_name() {
                remove_tree(&version_path)?;
            }
        }
    }
    let staging = store_directory(root, "staging")?;
    for entry in fs::read_dir(&staging).map_err(|source| io_error(&staging, source))? {
        let entry = entry.map_err(|source| io_error(&staging, source))?;
        count_store_entry(&mut entries_seen)?;
        remove_tree(&entry.path())?;
    }
    Ok(())
}

fn count_store_entry(count: &mut usize) -> Result<(), BundleError> {
    *count = count.saturating_add(1);
    if *count > MAX_PACKAGE_STORE_ENTRIES {
        return Err(BundleError::InvalidState(
            "extension package store contains too many entries",
        ));
    }
    Ok(())
}

pub(in crate::bundle) fn remove_extension_packages(
    root: &Path,
    id: &str,
) -> Result<(), BundleError> {
    if !valid_extension_id(id) {
        return Err(BundleError::InvalidState("extension ID is invalid"));
    }
    store_directory(root, "packages")?;
    remove_tree(&root.join("packages").join(id))
}

pub(in crate::bundle) fn ensure_data_directory(
    root: &Path,
    id: &str,
) -> Result<PathBuf, BundleError> {
    if !valid_extension_id(id) {
        return Err(BundleError::InvalidState("extension ID is invalid"));
    }
    let data = store_directory(root, "data")?;
    let path = data.join(id);
    ensure_directory(&path, 0o700)?;
    checked_directory(&path, &data)?.ok_or(BundleError::InvalidState(
        "extension data directory is missing after creation",
    ))
}

pub(in crate::bundle) fn data_directory(root: &Path, id: &str) -> Result<PathBuf, BundleError> {
    if !valid_extension_id(id) {
        return Err(BundleError::InvalidState("extension ID is invalid"));
    }
    let data = store_directory(root, "data")?;
    let path = data.join(id);
    match checked_directory(&path, &data)? {
        Some(directory) => Ok(directory),
        None => Ok(path),
    }
}

pub(in crate::bundle) fn remove_data_directory(root: &Path, id: &str) -> Result<(), BundleError> {
    if !valid_extension_id(id) {
        return Err(BundleError::InvalidState("extension ID is invalid"));
    }
    store_directory(root, "data")?;
    remove_tree(&root.join("data").join(id))
}
