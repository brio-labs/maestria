use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use maestria_extensions::{MAX_MANIFEST_BYTES, Manifest, ManifestError};
use thiserror::Error;

use crate::arguments::WorkerArguments;

pub(crate) const MAX_ENTRYPOINT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct ExtensionBundle {
    pub(crate) extension_id: String,
    pub(crate) module_name: String,
    pub(crate) command_ids: Vec<String>,
    pub(crate) source: String,
}

#[derive(Debug, Error)]
pub(crate) enum BundleError {
    #[error("bundle directory is unavailable: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("bundle root is not a directory")]
    RootNotDirectory,
    #[error("bundle has no root-level manifest.json or .extension.json file")]
    ManifestMissing,
    #[error("bundle has more than one possible manifest file")]
    ManifestAmbiguous,
    #[error("manifest file exceeds the worker manifest limit")]
    ManifestOversized,
    #[error("manifest resolves outside the sealed bundle root")]
    ManifestEscape,
    #[error("declared entrypoint was not found")]
    EntrypointMissing,
    #[error("declared entrypoint resolves outside the sealed bundle root")]
    EntrypointEscape,
    #[error("entrypoint source exceeds the worker source limit")]
    EntrypointOversized,
    #[error("entrypoint source is not UTF-8")]
    EntrypointEncoding,
}

pub(crate) fn load_bundle(arguments: &WorkerArguments) -> Result<ExtensionBundle, BundleError> {
    let root = arguments.bundle_root.canonicalize()?;
    if !root.is_dir() {
        return Err(BundleError::RootNotDirectory);
    }

    let manifest_path = find_manifest(&root)?;
    let manifest_path =
        confined_path(&root, &manifest_path).map_err(|_| BundleError::ManifestEscape)?;
    let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES, BundleFile::Manifest)?;
    let manifest = Manifest::parse(&manifest_bytes)?;
    let entrypoint = manifest
        .entrypoints
        .iter()
        .find(|entry| entry.id == arguments.entrypoint_id)
        .ok_or(BundleError::EntrypointMissing)?;
    let entrypoint_path = confined_path(&root, &root.join(&entrypoint.file))
        .map_err(|_| BundleError::EntrypointEscape)?;
    let source_bytes = read_bounded(
        &entrypoint_path,
        MAX_ENTRYPOINT_BYTES,
        BundleFile::Entrypoint,
    )?;
    let source = String::from_utf8(source_bytes).map_err(|_| BundleError::EntrypointEncoding)?;
    let command_ids = manifest
        .commands
        .iter()
        .filter(|command| command.entrypoint_id == entrypoint.id)
        .map(|command| command.id.clone())
        .collect::<Vec<_>>();

    Ok(ExtensionBundle {
        extension_id: manifest.id,
        module_name: entrypoint.file.clone(),
        command_ids,
        source,
    })
}

fn find_manifest(root: &Path) -> Result<PathBuf, BundleError> {
    let preferred = root.join("manifest.json");
    match fs::symlink_metadata(&preferred) {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            return Ok(preferred);
        }
        Ok(_) => return Err(BundleError::ManifestMissing),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(BundleError::Io(error)),
    }

    let mut candidate = None;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".extension.json") {
            if candidate.is_some() {
                return Err(BundleError::ManifestAmbiguous);
            }
            candidate = Some(entry.path());
        }
    }
    candidate.ok_or(BundleError::ManifestMissing)
}

fn confined_path(root: &Path, path: &Path) -> Result<PathBuf, io::Error> {
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "bundle path escapes root or is not a file",
        ));
    }
    Ok(canonical)
}

#[derive(Clone, Copy)]
enum BundleFile {
    Manifest,
    Entrypoint,
}

fn read_bounded(path: &Path, maximum: usize, kind: BundleFile) -> Result<Vec<u8>, BundleError> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > maximum as u64 {
        return Err(oversized_error(kind));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(oversized_error(kind));
    }
    Ok(bytes)
}

fn oversized_error(kind: BundleFile) -> BundleError {
    match kind {
        BundleFile::Manifest => BundleError::ManifestOversized,
        BundleFile::Entrypoint => BundleError::EntrypointOversized,
    }
}
