use anyhow::{Context, Result};
use sillage_core::{InstanceManifest, content_hash};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use super::watcher_state::FileSignature;
use crate::source_identity::source_key;

#[derive(Debug, Clone)]
pub(super) struct Observation {
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) hash: String,
}

#[derive(Debug, Default)]
pub(super) struct ScanResult {
    pub(super) observations: Vec<Observation>,
    pub(super) signatures: BTreeMap<String, FileSignature>,
    pub(super) admission_deferred_files: usize,
    pub(super) oversized_files: usize,
}

/// Scan manifest roots using `ignore::WalkBuilder` for gitignore/.ignore-aware
/// traversal. The walker respects `.gitignore`, `.ignore`, and hidden-file
/// conventions automatically.
fn is_instance_path(path: &Path, normalized_instance_root: &Path) -> bool {
    sillage_governance::lexical_normalize(path)
        .is_some_and(|normalized| normalized.starts_with(normalized_instance_root))
}

fn is_instance_internal_path(path: &Path, normalized_instance_root: &Path) -> bool {
    let Some(normalized_path) = sillage_governance::lexical_normalize(path) else {
        return false;
    };
    let Some(relative) = normalized_path.strip_prefix(normalized_instance_root).ok() else {
        return false;
    };
    let Some(first) = relative.components().next() else {
        return false;
    };
    matches!(
        first,
        Component::Normal(name)
            if matches!(name.to_str(), Some("system" | "indexes" | "blobs" | "manifest.txt"))
    )
}

pub(super) fn is_source_internal_path(
    normalized_root: &Path,
    path: &Path,
    normalized_instance_root: &Path,
) -> bool {
    if normalized_root != normalized_instance_root
        && normalized_instance_root.starts_with(normalized_root)
    {
        is_instance_path(path, normalized_instance_root)
    } else {
        is_instance_internal_path(path, normalized_instance_root)
    }
}

pub(super) fn file_signature(metadata: &fs::Metadata) -> FileSignature {
    let mtime = match metadata.modified() {
        Ok(time) => match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos() as i64,
            Err(_) => 0,
        },
        Err(_) => 0,
    };
    #[cfg(unix)]
    let change_time = {
        use std::os::unix::fs::MetadataExt;
        metadata
            .ctime()
            .saturating_mul(1_000_000_000)
            .saturating_add(metadata.ctime_nsec())
    };
    #[cfg(not(unix))]
    let change_time = mtime;
    FileSignature {
        mtime,
        change_time,
        size: metadata.len(),
    }
}

pub(super) fn scan_manifest(
    manifest: &InstanceManifest,
    previous: &BTreeMap<String, FileSignature>,
    recorded: &BTreeMap<String, String>,
    source_byte_budget: usize,
) -> Result<ScanResult> {
    let mut scan = ScanResult::default();
    let mut admitted_source_bytes = 0usize;
    let source_byte_budget = source_byte_budget.min(super::MAX_ADMITTED_SOURCE_BYTES);
    let instance_root = manifest.root.clone();
    let normalized_instance_root = match sillage_governance::lexical_normalize(&instance_root) {
        Some(normalized) => normalized,
        None => instance_root.clone(),
    };

    for root in &manifest.read_roots {
        let root = root.clone();
        let normalized_root = match sillage_governance::lexical_normalize(&root) {
            Some(normalized) => normalized,
            None => root.clone(),
        };
        let normalized_instance_root = normalized_instance_root.clone();
        let walker = ignore::WalkBuilder::new(root)
            .filter_entry(move |entry| {
                if is_source_internal_path(&normalized_root, entry.path(), &normalized_instance_root) {
                    return false;
                }
                if let Some(name) = entry.file_name().to_str() {
                    if sillage_index_selection::CANONICAL_EXCLUDED_NAMES.contains(&name) {
                        return false;
                    }
                }
                true
            })
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(false)
            .require_git(false)
            .follow_links(false)
            .build();
        for result in walker {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    continue;
                }
            };
            if entry.error().is_some() {
                continue;
            }
            let path = entry.path().to_path_buf();

            if !entry
                .file_type()
                .is_some_and(|ft| ft.is_file() && !ft.is_symlink())
            {
                continue;
            }

            if !manifest.allows_source(&path) {
                continue;
            }
            if sillage_index_selection::is_privacy_excluded_path(&path) {
                continue;
            }

            if !sillage_index_selection::is_supported_source_file(&path) {
                continue;
            }

            // Change detection: unchanged metadata signatures (mtime, ctime, size) are
            // not re-read; their recorded content hash is reused by the
            // caller. A file is only skipped when a recorded hash exists:
            // sources that were never durably accepted (e.g. pending at
            // shutdown) must be re-read so they are delivered after a
            // restart (issue #440).
            let key = source_key(&path);
            let mut file = match fs::File::open(&path) {
                Ok(file) => file,
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        || error.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("open watched file {}", path.display()));
                }
            };
            let metadata = match file.metadata() {
                Ok(metadata) => metadata,
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        || error.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("stat watched file {}", path.display()));
                }
            };
            if !metadata.is_file() {
                continue;
            }
            let signature = file_signature(&metadata);
            if previous.get(&key) == Some(&signature) && recorded.contains_key(&key) {
                scan.signatures.insert(key, signature);
                continue;
            }

            let remaining_bytes = source_byte_budget.saturating_sub(admitted_source_bytes);
            if signature.size > remaining_bytes as u64 {
                scan.admission_deferred_files += 1;
                if signature.size > super::MAX_ADMITTED_SOURCE_BYTES as u64 {
                    scan.oversized_files += 1;
                }
                continue;
            }

            let capacity = usize::try_from(signature.size)
                .unwrap_or(remaining_bytes)
                .min(remaining_bytes);
            let mut bytes = Vec::with_capacity(capacity);
            // Limit the read to the statted size so concurrent growth cannot
            // expand this batch past the reserved source-byte budget.
            if let Err(error) = (&mut file).take(signature.size).read_to_end(&mut bytes) {
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.kind() == std::io::ErrorKind::NotFound
                {
                    continue;
                }
                return Err(error).with_context(|| format!("read watched file {}", path.display()));
            }
            let handle_signature = match file.metadata() {
                Ok(metadata) => file_signature(&metadata),
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        || error.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("re-stat watched file {}", path.display()));
                }
            };
            let path_signature = match fs::metadata(&path) {
                Ok(metadata) => file_signature(&metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    scan.admission_deferred_files += 1;
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("re-stat watched file {}", path.display())
                    });
                }
            };
            if bytes.len() as u64 != signature.size
                || handle_signature != signature
                || path_signature != signature
            {
                scan.admission_deferred_files += 1;
                continue;
            }

            admitted_source_bytes = admitted_source_bytes.saturating_add(bytes.len());
            scan.observations.push(Observation {
                path,
                hash: content_hash(&bytes),
                bytes,
            });
            scan.signatures.insert(key, signature);
        }
    }
    scan.observations.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(scan)
}
