use anyhow::{Context, Result};
use maestria_core::{InstanceManifest, content_hash};
use std::{
    collections::BTreeMap,
    fs,
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

/// Scan manifest roots using `ignore::WalkBuilder` for gitignore/.ignore-aware
/// traversal. The walker respects `.gitignore`, `.ignore`, and hidden-file
/// conventions automatically.
fn is_instance_path(path: &Path, normalized_instance_root: &Path) -> bool {
    maestria_governance::lexical_normalize(path)
        .is_some_and(|normalized| normalized.starts_with(normalized_instance_root))
}

fn is_instance_internal_path(path: &Path, normalized_instance_root: &Path) -> bool {
    let Some(normalized_path) = maestria_governance::lexical_normalize(path) else {
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

pub(super) fn scan_manifest(
    manifest: &InstanceManifest,
    previous: &BTreeMap<String, FileSignature>,
    recorded: &BTreeMap<String, String>,
) -> Result<(Vec<Observation>, BTreeMap<String, FileSignature>)> {
    let mut observations = Vec::new();
    let mut signatures = BTreeMap::new();
    let instance_root = manifest.root.clone();
    let normalized_instance_root = match maestria_governance::lexical_normalize(&instance_root) {
        Some(normalized) => normalized,
        None => instance_root.clone(),
    };

    for root in &manifest.read_roots {
        let root = root.clone();
        let normalized_root = match maestria_governance::lexical_normalize(&root) {
            Some(normalized) => normalized,
            None => root.clone(),
        };
        let exclude_instance = normalized_root != normalized_instance_root
            && normalized_instance_root.starts_with(&normalized_root);
        let normalized_instance_root = normalized_instance_root.clone();
        let walker = ignore::WalkBuilder::new(root)
            .filter_entry(move |entry| {
                if exclude_instance {
                    !is_instance_path(entry.path(), &normalized_instance_root)
                } else {
                    !is_instance_internal_path(entry.path(), &normalized_instance_root)
                }
            })
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(false)
            .require_git(false)
            .follow_links(false)
            .build();
        for result in walker {
            let entry = result?;
            if let Some(error) = entry.error() {
                return Err(anyhow::anyhow!(
                    "traversal error at {}: {error}",
                    entry.path().display()
                ));
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

            if !maestria_index_selection::is_supported_source_file(&path) {
                continue;
            }

            // Change detection: unchanged files (same mtime and size) are
            // not re-read; their recorded content hash is reused by the
            // caller. A file is only skipped when a recorded hash exists:
            // sources that were never durably accepted (e.g. pending at
            // shutdown) must be re-read so they are delivered after a
            // restart (issue #440).
            let key = source_key(&path);
            let metadata = fs::metadata(&path)
                .with_context(|| format!("stat watched file {}", path.display()))?;
            let signature = FileSignature {
                mtime: match metadata.modified() {
                    Ok(time) => match time.duration_since(std::time::UNIX_EPOCH) {
                        Ok(duration) => duration.as_nanos() as i64,
                        Err(_) => 0,
                    },
                    Err(_) => 0,
                },
                size: metadata.len(),
            };
            if previous.get(&key) == Some(&signature) && recorded.contains_key(&key) {
                signatures.insert(key, signature);
                continue;
            }

            let bytes =
                fs::read(&path).with_context(|| format!("read watched file {}", path.display()))?;
            observations.push(Observation {
                path,
                hash: content_hash(&bytes),
                bytes,
            });
            signatures.insert(key, signature);
        }
    }
    observations.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((observations, signatures))
}
