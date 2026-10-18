use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Manifest, ManifestError, Permission};

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("extension bundle I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid extension package: {0}")]
    InvalidPackage(String),
    #[error("invalid extension manifest: {0}")]
    Manifest(#[from] ManifestError),
    #[error("invalid extension store state: {0}")]
    InvalidState(&'static str),
    #[error("unsupported extension store schema version {0}")]
    StateVersion(u32),
    #[error("invalid extension store JSON: {0}")]
    StateJson(#[from] serde_json::Error),
    #[error("state commit completed but directory durability could not be confirmed: {0}")]
    StateCommitted(#[source] std::io::Error),
    #[error("installation committed but package cleanup failed: {0}")]
    PostCommitCleanup(#[source] Box<BundleError>),
    #[error("cleanup failed after {operation}: cause={cause}; cleanup={cleanup}")]
    Cleanup {
        operation: &'static str,
        #[source]
        cause: Box<BundleError>,
        cleanup: std::io::Error,
    },
    #[error("invalid ZIP extension archive: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("extension installation was not approved")]
    ApprovalDenied,
    #[error("extension store changed while installation approval was pending")]
    StoreChanged,
    #[error("extension {0:?} is not installed")]
    NotInstalled(String),
    #[error("extension package identity conflicts with an existing sealed package")]
    IdentityConflict,
    #[error("extension store lock is poisoned")]
    LockPoisoned,
    #[error(
        "extension install failed and its staged package could not be removed: install={install}; cleanup={cleanup}"
    )]
    InstallCleanup {
        #[source]
        install: Box<BundleError>,
        cleanup: Box<BundleError>,
    },
    #[error(
        "extension state commit failed and its unreferenced package could not be removed: commit={commit}; cleanup={cleanup}"
    )]
    CommitCleanup {
        #[source]
        commit: Box<BundleError>,
        cleanup: Box<BundleError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageIdentity {
    pub version: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDiff {
    pub previously_granted: Vec<Permission>,
    pub requested: Vec<Permission>,
    pub added: Vec<Permission>,
    pub removed: Vec<Permission>,
}

#[derive(Debug, Clone)]
pub struct InstallApproval {
    pub manifest: Manifest,
    pub package: PackageIdentity,
    pub permissions: PermissionDiff,
    pub first_install: bool,
}

impl InstallApproval {
    /// Compares every manifest and approval field for preview-to-install checks.
    pub fn matches(&self, other: &Self) -> bool {
        self.package == other.package
            && self.permissions == other.permissions
            && self.first_install == other.first_install
            && manifests_match(&self.manifest, &other.manifest)
    }
}

fn manifests_match(left: &Manifest, right: &Manifest) -> bool {
    left.api_version == right.api_version
        && left.id == right.id
        && left.name == right.name
        && left.version == right.version
        && left.permissions == right.permissions
        && left.entrypoints.len() == right.entrypoints.len()
        && left
            .entrypoints
            .iter()
            .zip(&right.entrypoints)
            .all(|(left, right)| left.id == right.id && left.file == right.file)
        && left.commands.len() == right.commands.len()
        && left
            .commands
            .iter()
            .zip(&right.commands)
            .all(|(left, right)| {
                left.id == right.id
                    && left.title == right.title
                    && left.entrypoint_id == right.entrypoint_id
                    && left.description == right.description
            })
}

#[derive(Debug, Clone)]
pub struct InstallReceipt {
    pub extension_id: String,
    pub package: PackageIdentity,
    pub newly_granted: Vec<Permission>,
    pub activated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionHealth {
    Validated,
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct ExtensionSummary {
    pub extension_id: String,
    pub name: String,
    pub package: PackageIdentity,
    pub enabled: bool,
    pub requested_permissions: Vec<Permission>,
    pub granted_permissions: Vec<Permission>,
    pub health: ExtensionHealth,
}

#[derive(Debug, Clone)]
pub struct ActiveBundle {
    pub extension_id: String,
    pub manifest: Manifest,
    pub package: PackageIdentity,
    pub code_directory: PathBuf,
    pub data_directory: PathBuf,
    pub granted_permissions: Vec<Permission>,
}
