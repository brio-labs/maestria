use std::path::{Path, PathBuf};

#[path = "bundle/archive.rs"]
mod archive;
#[path = "bundle/install.rs"]
mod install;
#[path = "bundle/lock.rs"]
mod lock;
#[path = "bundle/persistence.rs"]
mod persistence;
#[path = "bundle/source.rs"]
mod source;
#[path = "bundle/types.rs"]
mod types;
pub use types::{
    ActiveBundle, BundleError, ExtensionHealth, ExtensionSummary, InstallApproval, InstallReceipt,
    PackageIdentity, PermissionDiff,
};

pub const MAX_PACKAGE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_PACKAGE_FILES: usize = 65;

/// Owns validated, sealed extension packages and durable lifecycle state.
/// This type never loads or executes third-party code.
pub struct BundleStore {
    root: PathBuf,
}

impl BundleStore {
    pub fn open(root: PathBuf) -> Result<Self, BundleError> {
        let root = persistence::resolve_root(root)?;
        let store = Self { root };
        let _guard = lock::store_lock()?;
        persistence::load_state(&store.root)?;
        Ok(store)
    }

    /// Validates and installs a directory package. The approval callback runs
    /// only after package validation and receives the exact grant expansion.
    pub fn install_directory(
        &self,
        source_path: &Path,
        approve: impl FnOnce(&InstallApproval) -> bool,
    ) -> Result<InstallReceipt, BundleError> {
        let files = source::read_directory(source_path)?;
        install::install_files(self, files, approve)
    }

    /// Validates and installs a local ZIP package without extracting untrusted
    /// archive paths directly onto the filesystem.
    pub fn install_archive(
        &self,
        archive_path: &Path,
        approve: impl FnOnce(&InstallApproval) -> bool,
    ) -> Result<InstallReceipt, BundleError> {
        let files = archive::read_archive(archive_path)?;
        install::install_files(self, files, approve)
    }

    /// Produces a validated proposal without writing package or extension data.
    pub fn preview_directory(&self, source_path: &Path) -> Result<InstallApproval, BundleError> {
        let files = source::read_directory(source_path)?;
        install::preview_files(self, files)
    }

    /// Produces a validated proposal without writing package or extension data.
    pub fn preview_archive(&self, archive_path: &Path) -> Result<InstallApproval, BundleError> {
        let files = archive::read_archive(archive_path)?;
        install::preview_files(self, files)
    }

    /// Lists recoverable records. Per-package integrity problems are returned
    /// as explicit health status so the UI can offer disable, revoke, or remove.
    pub fn list(&self) -> Result<Vec<ExtensionSummary>, BundleError> {
        let _guard = lock::store_lock()?;
        let state = persistence::load_state(&self.root)?;
        let mut summaries = Vec::with_capacity(state.installations.len());
        for installation in &state.installations {
            let health = match install::read_installation(self, installation) {
                Ok(_) => ExtensionHealth::Validated,
                Err(error) => ExtensionHealth::Invalid(error.to_string()),
            };
            summaries.push(ExtensionSummary {
                extension_id: installation.extension_id.clone(),
                name: installation.name.clone(),
                package: installation.package.clone(),
                enabled: installation.enabled,
                requested_permissions: installation.requested_permissions.clone(),
                granted_permissions: installation.granted_permissions.clone(),
                health,
            });
        }
        Ok(summaries)
    }

    /// Returns a fully revalidated package only when it is enabled. No code is
    /// executed here; the caller remains responsible for OS isolation first.
    /// `data_directory` is a host path; the broker must check grants before exposing it.
    pub fn active_bundle(&self, id: &str) -> Result<Option<ActiveBundle>, BundleError> {
        let _guard = lock::store_lock()?;
        let state = persistence::load_state(&self.root)?;
        let Some(installation) = persistence::installation(&state, id) else {
            return Ok(None);
        };
        if !installation.enabled {
            return Ok(None);
        }
        let (manifest, package) = install::read_installation(self, installation)?;
        Ok(Some(ActiveBundle {
            extension_id: installation.extension_id.clone(),
            manifest,
            package,
            code_directory: persistence::package_directory(&self.root, installation)?,
            data_directory: persistence::data_directory(&self.root, id)?,
            granted_permissions: installation.granted_permissions.clone(),
        }))
    }

    /// Disables commands immediately while retaining the package and grants.
    pub fn disable(&self, id: &str) -> Result<(), BundleError> {
        self.update_installation(id, |record| record.enabled = false)
    }

    /// Disables commands and clears all grants for the active package version.
    pub fn revoke(&self, id: &str) -> Result<(), BundleError> {
        self.update_installation(id, |record| {
            record.enabled = false;
            record.granted_permissions.clear();
        })
    }

    /// Uninstalls and deletes extension-local data unless explicitly retained.
    pub fn uninstall_default(&self, id: &str) -> Result<(), BundleError> {
        self.uninstall(id, false)
    }

    /// Removes code and grants, retaining extension-local data only when true.
    pub fn uninstall(&self, id: &str, retain_data: bool) -> Result<(), BundleError> {
        let _guard = lock::store_lock()?;
        let Some(file_lock) = lock::interprocess_store_lock(&self.root)? else {
            return Err(BundleError::NotInstalled(id.to_owned()));
        };
        let store_root = file_lock.root_path(&self.root);
        let mut state = persistence::load_state(&store_root)?;
        let index = persistence::installation_index(&state, id)
            .ok_or_else(|| BundleError::NotInstalled(id.to_owned()))?;
        let mut disabled = state
            .installations
            .get(index)
            .cloned()
            .ok_or(BundleError::InvalidState("installation index is invalid"))?;
        disabled.enabled = false;
        disabled.granted_permissions.clear();
        persistence::set_installation(&mut state, index, disabled.clone())?;
        bump_generation(&mut state)?;
        persistence::save_state(&store_root, &state)?;

        persistence::remove_extension_packages(&store_root, id)?;
        if !retain_data {
            persistence::remove_data_directory(&store_root, id)?;
        }
        state
            .installations
            .retain(|record| record.extension_id != id);
        bump_generation(&mut state)?;
        persistence::save_state(&store_root, &state)
    }

    fn update_installation(
        &self,
        id: &str,
        update: impl FnOnce(&mut persistence::InstallationRecord),
    ) -> Result<(), BundleError> {
        let _guard = lock::store_lock()?;
        let Some(file_lock) = lock::interprocess_store_lock(&self.root)? else {
            return Err(BundleError::NotInstalled(id.to_owned()));
        };
        let store_root = file_lock.root_path(&self.root);
        let mut state = persistence::load_state(&store_root)?;
        let index = persistence::installation_index(&state, id)
            .ok_or_else(|| BundleError::NotInstalled(id.to_owned()))?;
        let mut record = state
            .installations
            .get(index)
            .cloned()
            .ok_or(BundleError::InvalidState("installation index is invalid"))?;
        update(&mut record);
        persistence::set_installation(&mut state, index, record)?;
        bump_generation(&mut state)?;
        persistence::save_state(&store_root, &state)
    }
}

fn bump_generation(state: &mut persistence::StoreState) -> Result<(), BundleError> {
    state.generation = state
        .generation
        .checked_add(1)
        .ok_or(BundleError::InvalidState("generation counter overflow"))?;
    Ok(())
}
