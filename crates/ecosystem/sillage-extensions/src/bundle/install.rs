use std::collections::BTreeSet;

use crate::{Manifest, Permission};

use super::lock;
use super::persistence;
use super::source;
use super::{
    BundleError, BundleStore, InstallApproval, InstallReceipt, PackageIdentity, PermissionDiff,
};

struct PreparedInstall {
    files: source::PackageFiles,
    manifest: Manifest,
    package: PackageIdentity,
    permissions: PermissionDiff,
    first_install: bool,
    generation: u64,
}

impl PreparedInstall {
    fn approval(&self) -> InstallApproval {
        InstallApproval {
            manifest: self.manifest.clone(),
            package: self.package.clone(),
            permissions: self.permissions.clone(),
            first_install: self.first_install,
        }
    }
}

pub(super) fn install_files(
    store: &BundleStore,
    files: source::PackageFiles,
    approve: impl FnOnce(&InstallApproval) -> bool,
) -> Result<InstallReceipt, BundleError> {
    let prepared = prepare_install(store, files)?;
    let approval = prepared.approval();
    if !approve(&approval) {
        return Err(BundleError::ApprovalDenied);
    }
    let PreparedInstall {
        files,
        manifest,
        package,
        permissions,
        generation,
        ..
    } = prepared;

    let _guard = lock::store_lock()?;
    let initialized_root = persistence::initialize_root(store.root.clone())?;
    if initialized_root.as_path() != store.root.as_path() {
        return Err(BundleError::InvalidState(
            "extension store root changed while approval was pending",
        ));
    }
    let file_lock = lock::interprocess_store_lock(&store.root)?.ok_or_else(|| {
        persistence::io_error(
            &store.root,
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "extension store root disappeared before locking",
            ),
        )
    })?;
    let store_root = file_lock.root_path(&store.root);
    let mut state = persistence::load_state(&store_root)?;
    if state.generation != generation {
        return Err(BundleError::StoreChanged);
    }
    let record = persistence::InstallationRecord {
        extension_id: manifest.id.clone(),
        name: manifest.name.clone(),
        package: package.clone(),
        grants_for: package.clone(),
        enabled: true,
        requested_permissions: manifest.permissions.clone(),
        granted_permissions: manifest.permissions.clone(),
    };
    persistence::replace_installation(&mut state, record)?;
    state.generation = generation
        .checked_add(1)
        .ok_or(BundleError::InvalidState("generation counter overflow"))?;
    persistence::ensure_data_directory(&store_root, &manifest.id)?;
    let (sealed_path, created) =
        persistence::seal_package(&store_root, &manifest.id, &files, &package)?;
    if let Err(commit) = persistence::save_state(&store_root, &state) {
        if created
            && !matches!(&commit, BundleError::StateCommitted(_))
            && let Err(cleanup) = persistence::remove_sealed_package(&sealed_path)
        {
            return Err(BundleError::CommitCleanup {
                commit: Box::new(commit),
                cleanup: Box::new(cleanup),
            });
        }
        return Err(commit);
    }
    if let Err(cleanup) = persistence::prune_unreferenced_packages(&store_root, &state) {
        return Err(BundleError::PostCommitCleanup(Box::new(cleanup)));
    }
    Ok(InstallReceipt {
        extension_id: manifest.id,
        package,
        newly_granted: permissions.added,
        activated: true,
    })
}

pub(super) fn preview_files(
    store: &BundleStore,
    files: source::PackageFiles,
) -> Result<InstallApproval, BundleError> {
    Ok(prepare_install(store, files)?.approval())
}

fn prepare_install(
    store: &BundleStore,
    files: source::PackageFiles,
) -> Result<PreparedInstall, BundleError> {
    let manifest = validate_package_files(&files)?;
    let package = PackageIdentity {
        version: manifest.version.clone(),
        sha256: persistence::digest_files(&files),
    };
    let (generation, prior) = {
        let _guard = lock::store_lock()?;
        let state = persistence::load_state(&store.root)?;
        let prior = persistence::installation(&state, &manifest.id).cloned();
        (state.generation, prior)
    };
    let previously_granted = match prior.as_ref() {
        Some(record) => record.granted_permissions.clone(),
        None => Vec::new(),
    };
    let permissions = permission_diff(previously_granted, manifest.permissions.clone());
    Ok(PreparedInstall {
        files,
        manifest,
        package,
        permissions,
        first_install: prior.is_none(),
        generation,
    })
}

pub(super) fn read_installation(
    store: &BundleStore,
    installation: &persistence::InstallationRecord,
) -> Result<(Manifest, PackageIdentity), BundleError> {
    let directory = persistence::package_directory(&store.root, installation)?;
    let files = source::read_directory(&directory)?;
    let manifest = validate_package_files(&files)?;
    let digest = persistence::digest_files(&files);
    if manifest.id != installation.extension_id
        || manifest.version != installation.package.version
        || digest != installation.package.sha256
    {
        return Err(BundleError::InvalidPackage(
            "sealed package identity does not match durable installation state".to_owned(),
        ));
    }
    if manifest.name != installation.name
        || manifest.permissions != installation.requested_permissions
    {
        return Err(BundleError::InvalidState(
            "stored onboarding details do not match the sealed manifest",
        ));
    }
    for granted in &installation.granted_permissions {
        if !manifest.permissions.contains(granted) {
            return Err(BundleError::InvalidState(
                "stored grant is not requested by the active manifest",
            ));
        }
    }
    Ok((manifest, installation.package.clone()))
}

fn validate_package_files(files: &source::PackageFiles) -> Result<Manifest, BundleError> {
    if files.is_empty() || files.len() > super::MAX_PACKAGE_FILES {
        return Err(BundleError::InvalidPackage(
            "expected a manifest and at most 64 declared files".to_owned(),
        ));
    }
    let manifest_files: Vec<&String> = files
        .keys()
        .filter(|path| path.as_str() == "manifest.json" || path.ends_with(".extension.json"))
        .collect();
    if manifest_files.len() != 1 {
        return Err(BundleError::InvalidPackage(
            "expected exactly one root manifest.json or *.extension.json file".to_owned(),
        ));
    }
    let manifest_file = manifest_files
        .first()
        .ok_or_else(|| BundleError::InvalidPackage("manifest file is missing".to_owned()))?
        .to_string();
    if manifest_file.contains('/') {
        return Err(BundleError::InvalidPackage(
            "the extension manifest must be at the package root".to_owned(),
        ));
    }
    let manifest_bytes = files
        .get(&manifest_file)
        .ok_or_else(|| BundleError::InvalidPackage("manifest file is missing".to_owned()))?;
    let manifest = Manifest::parse(manifest_bytes)?;
    let mut declared = BTreeSet::new();
    declared.insert(manifest_file.clone());
    for entrypoint in &manifest.entrypoints {
        declared.insert(entrypoint.file.clone());
    }
    if files.keys().any(|path| !declared.contains(path)) {
        return Err(BundleError::InvalidPackage(
            "package contains a file not declared by its manifest".to_owned(),
        ));
    }
    if declared.len() != files.len() {
        return Err(BundleError::InvalidPackage(
            "package is missing a declared entrypoint file".to_owned(),
        ));
    }
    Ok(manifest)
}

fn permission_diff(
    previously_granted: Vec<Permission>,
    requested: Vec<Permission>,
) -> PermissionDiff {
    let added = requested
        .iter()
        .filter(|permission| !previously_granted.contains(permission))
        .cloned()
        .collect();
    let removed = previously_granted
        .iter()
        .filter(|permission| !requested.contains(permission))
        .cloned()
        .collect();
    PermissionDiff {
        previously_granted,
        requested,
        added,
        removed,
    }
}
