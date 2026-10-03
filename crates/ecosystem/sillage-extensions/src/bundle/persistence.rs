use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::PackageIdentity;
use crate::Permission;
use crate::bundle::BundleError;
use serde::{Deserialize, Serialize};

#[path = "persistence_package.rs"]
mod package;
pub(super) use package::{
    data_directory, digest_files, ensure_data_directory, package_directory,
    prune_unreferenced_packages, remove_data_directory, remove_extension_packages, seal_package,
};
#[path = "persistence_fs.rs"]
mod filesystem;
pub(super) use filesystem::{
    checked_directory, ensure_directory, io_error, remove_sealed_package, remove_tree,
    set_directory_permissions, set_file_permissions, store_directory, sync_directory,
};

const STATE_SCHEMA_VERSION: u32 = 1;
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INSTALLATIONS: usize = 1024;
const STAGING_ATTEMPTS: usize = 16;
static TEMPORARY_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoreState {
    pub(super) schema_version: u32,
    pub(super) generation: u64,
    pub(super) installations: Vec<InstallationRecord>,
}

impl StoreState {
    fn empty() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            generation: 0,
            installations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct InstallationRecord {
    pub(super) extension_id: String,
    pub(super) name: String,
    pub(super) package: PackageIdentity,
    pub(super) grants_for: PackageIdentity,
    pub(super) enabled: bool,
    pub(super) requested_permissions: Vec<Permission>,
    pub(super) granted_permissions: Vec<Permission>,
}

pub(super) fn resolve_root(path: PathBuf) -> Result<PathBuf, BundleError> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|source| io_error(&path, source))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(name) => normalized.push(name),
        }
    }

    let mut candidate = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(candidate) {
            Ok(metadata)
                if candidate == normalized.as_path() && metadata.file_type().is_symlink() =>
            {
                return Err(BundleError::InvalidState(
                    "extension store root must not be a symbolic link",
                ));
            }
            Ok(_) => {
                let mut resolved =
                    fs::canonicalize(candidate).map_err(|source| io_error(candidate, source))?;
                if !fs::metadata(&resolved)
                    .map_err(|source| io_error(candidate, source))?
                    .is_dir()
                {
                    return Err(BundleError::InvalidState(
                        "extension store root must be a real directory",
                    ));
                }
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let component = candidate.file_name().ok_or(BundleError::InvalidState(
                    "extension store root has no existing directory ancestor",
                ))?;
                missing.push(component.to_os_string());
                candidate = candidate.parent().ok_or(BundleError::InvalidState(
                    "extension store root has no existing directory ancestor",
                ))?;
            }
            Err(source) => return Err(io_error(candidate, source)),
        }
    }
}

pub(super) fn initialize_root(path: PathBuf) -> Result<PathBuf, BundleError> {
    if resolve_root(path.clone())?.as_path() != path.as_path() {
        return Err(BundleError::InvalidState(
            "extension store root changed after resolution",
        ));
    }
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BundleError::InvalidState(
                "extension store root must be a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(&path).map_err(|source| io_error(&path, source))?;
        }
        Err(source) => return Err(io_error(&path, source)),
    }
    let canonical_root = fs::canonicalize(&path).map_err(|source| io_error(&path, source))?;
    set_directory_permissions(&canonical_root, 0o700)?;
    for child in ["packages", "staging", "data"] {
        ensure_directory(&canonical_root.join(child), 0o700)?;
    }
    Ok(canonical_root)
}

pub(super) fn load_state(root: &Path) -> Result<StoreState, BundleError> {
    let path = root.join("state.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(StoreState::empty()),
        Err(source) => return Err(io_error(&path, source)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BundleError::InvalidState(
            "state file must be a regular non-symbolic-link file",
        ));
    }
    let bytes = read_state_file(&path)?;
    let state: StoreState = serde_json::from_slice(&bytes)?;
    validate_state(&state)?;
    Ok(state)
}

fn read_state_file(path: &Path) -> Result<Vec<u8>, BundleError> {
    let file = crate::bundle::source::open_regular_file(path)?;
    let metadata = file.metadata().map_err(|source| io_error(path, source))?;
    if !metadata.is_file() || metadata.len() > MAX_STATE_BYTES {
        return Err(BundleError::InvalidState(
            "state file is not a bounded regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(BundleError::InvalidState(
            "state file exceeds its size limit",
        ));
    }
    Ok(bytes)
}
pub(super) fn save_state(root: &Path, state: &StoreState) -> Result<(), BundleError> {
    validate_state(state)?;
    let bytes = serde_json::to_vec(state)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(BundleError::InvalidState(
            "state file exceeds its size limit",
        ));
    }
    let state_path = root.join("state.json");
    ensure_state_target(&state_path)?;
    let temporary_path = create_temporary_state(root, &bytes)?;
    match fs::rename(&temporary_path, &state_path) {
        Ok(()) => {}
        Err(source) => {
            let commit = io_error(&state_path, source);
            return match fs::remove_file(&temporary_path) {
                Ok(()) => Err(commit),
                Err(cleanup) => Err(BundleError::Cleanup {
                    operation: "replace state file",
                    cause: Box::new(commit),
                    cleanup,
                }),
            };
        }
    }
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(BundleError::StateCommitted)
}

fn create_temporary_state(root: &Path, bytes: &[u8]) -> Result<PathBuf, BundleError> {
    for _ in 0..STAGING_ATTEMPTS {
        let sequence = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".state-{}-{sequence}", std::process::id()));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&path, source)),
        };
        let write_result = file
            .write_all(bytes)
            .and_then(|()| set_file_permissions(&path, 0o600))
            .and_then(|()| file.sync_all());
        if let Err(source) = write_result {
            let primary = io_error(&path, source);
            return match fs::remove_file(&path) {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(BundleError::Cleanup {
                    operation: "write temporary state file",
                    cause: Box::new(primary),
                    cleanup,
                }),
            };
        }
        return Ok(path);
    }
    Err(BundleError::InvalidState(
        "could not allocate a unique temporary state file",
    ))
}

fn ensure_state_target(path: &Path) -> Result<(), BundleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            BundleError::InvalidState("state target must be a regular non-symbolic-link file"),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn validate_state(state: &StoreState) -> Result<(), BundleError> {
    if state.schema_version != STATE_SCHEMA_VERSION {
        return Err(BundleError::StateVersion(state.schema_version));
    }
    if state.installations.len() > MAX_INSTALLATIONS {
        return Err(BundleError::InvalidState(
            "too many installed extension records",
        ));
    }
    let mut ids = BTreeSet::new();
    for record in &state.installations {
        if !valid_extension_id(&record.extension_id)
            || !ids.insert(&record.extension_id)
            || !valid_display_name(&record.name)
        {
            return Err(BundleError::InvalidState(
                "installation IDs or display names are invalid or duplicated",
            ));
        }
        validate_package_identity(&record.package)?;
        validate_package_identity(&record.grants_for)?;
        if record.grants_for != record.package {
            return Err(BundleError::InvalidState(
                "grants are not scoped to the active package identity",
            ));
        }
        validate_permission_set(&record.requested_permissions)?;
        validate_permission_set(&record.granted_permissions)?;
        if record
            .granted_permissions
            .iter()
            .any(|permission| !record.requested_permissions.contains(permission))
        {
            return Err(BundleError::InvalidState(
                "stored grants exceed the active manifest request",
            ));
        }
    }
    Ok(())
}

fn valid_display_name(name: &str) -> bool {
    !name.is_empty()
        && name.encode_utf16().take(81).count() <= 80
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

fn validate_permission_set(permissions: &[Permission]) -> Result<(), BundleError> {
    if permissions.len() > 7 {
        return Err(BundleError::InvalidState("too many stored permissions"));
    }
    let mut kinds = BTreeSet::new();
    if permissions
        .iter()
        .any(|permission| !kinds.insert(permission.kind()))
    {
        return Err(BundleError::InvalidState(
            "stored permissions contain duplicate capability types",
        ));
    }
    Ok(())
}

fn valid_extension_id(id: &str) -> bool {
    id.len() <= 128
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && id.split(['.', '-']).all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn validate_package_identity(identity: &PackageIdentity) -> Result<(), BundleError> {
    if identity.version.len() > 64 || semver::Version::parse(&identity.version).is_err() {
        return Err(BundleError::InvalidState(
            "package identity has an invalid semantic version",
        ));
    }
    if identity.sha256.len() != 64
        || !identity
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(BundleError::InvalidState(
            "package identity has an invalid SHA-256 digest",
        ));
    }
    Ok(())
}

pub(super) fn installation<'a>(state: &'a StoreState, id: &str) -> Option<&'a InstallationRecord> {
    state
        .installations
        .iter()
        .find(|record| record.extension_id == id)
}

pub(super) fn installation_index(state: &StoreState, id: &str) -> Option<usize> {
    state
        .installations
        .iter()
        .position(|record| record.extension_id == id)
}

pub(super) fn replace_installation(
    state: &mut StoreState,
    installation: InstallationRecord,
) -> Result<(), BundleError> {
    if let Some(index) = installation_index(state, &installation.extension_id) {
        set_installation(state, index, installation)
    } else {
        if state.installations.len() >= MAX_INSTALLATIONS {
            return Err(BundleError::InvalidState(
                "extension store has reached its installation limit",
            ));
        }
        state.installations.push(installation);
        state
            .installations
            .sort_by(|left, right| left.extension_id.cmp(&right.extension_id));
        Ok(())
    }
}

pub(super) fn set_installation(
    state: &mut StoreState,
    index: usize,
    installation: InstallationRecord,
) -> Result<(), BundleError> {
    let target = state
        .installations
        .get_mut(index)
        .ok_or(BundleError::InvalidState("installation index is invalid"))?;
    *target = installation;
    Ok(())
}
