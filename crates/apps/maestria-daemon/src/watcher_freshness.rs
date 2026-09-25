use std::{collections::BTreeSet, io::Read, path::Path};

use maestria_core::{InstanceLayout, InstanceManifest};

use super::watcher_scan;
#[cfg(unix)]
use super::watcher_state::load_state;
#[cfg(unix)]
use crate::source_identity::source_key;

pub(crate) fn is_internal_source_path(
    manifest: &InstanceManifest,
    root: &Path,
    path: &Path,
) -> bool {
    let normalized_root = match maestria_governance::lexical_normalize(root) {
        Some(normalized) => normalized,
        None => root.to_path_buf(),
    };
    let normalized_instance_root = match maestria_governance::lexical_normalize(&manifest.root) {
        Some(normalized) => normalized,
        None => manifest.root.clone(),
    };
    watcher_scan::is_source_internal_path(&normalized_root, path, &normalized_instance_root)
}

pub(crate) fn fresh_source_paths(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
    sources: &[(String, String)],
) -> BTreeSet<String> {
    fresh_source_paths_with_byte_budget(layout, manifest, sources, None)
}

pub(crate) fn fresh_source_paths_bounded(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
    sources: &[(String, String)],
    max_rehash_bytes: u64,
) -> BTreeSet<String> {
    fresh_source_paths_with_byte_budget(layout, manifest, sources, Some(max_rehash_bytes))
}

fn fresh_source_paths_with_byte_budget(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
    sources: &[(String, String)],
    mut rehash_bytes_remaining: Option<u64>,
) -> BTreeSet<String> {
    #[cfg(unix)]
    let state = rehash_bytes_remaining.is_none().then(|| load_state(layout));
    sources
        .iter()
        .filter_map(|(source, expected_hash)| {
            let source_path = Path::new(source);
            let path = if source_path.is_absolute() {
                source_path.to_path_buf()
            } else {
                layout.root.join(source_path)
            };
            let normalized = maestria_governance::lexical_normalize(&path)?;
            if !manifest.allows_source(&normalized)
                || maestria_index_selection::is_privacy_excluded_path(&normalized)
            {
                return None;
            }
            let lexical_root = manifest
                .read_roots
                .iter()
                .filter_map(|root| maestria_governance::lexical_normalize(root))
                .filter(|root| normalized.starts_with(root))
                .max_by_key(|root| root.components().count())?;
            if is_internal_source_path(manifest, &lexical_root, &normalized) {
                return None;
            }
            if std::fs::symlink_metadata(&lexical_root)
                .ok()?
                .file_type()
                .is_symlink()
            {
                return None;
            }
            let canonical_root = lexical_root.canonicalize().ok()?;
            let canonical = path.canonicalize().ok()?;
            if !canonical.starts_with(&canonical_root)
                || maestria_index_selection::is_privacy_excluded_path(&canonical)
            {
                return None;
            }
            let relative = normalized.strip_prefix(&lexical_root).ok()?;
            let mut current = lexical_root;
            for component in relative.components() {
                current.push(component.as_os_str());
                if std::fs::symlink_metadata(&current)
                    .ok()?
                    .file_type()
                    .is_symlink()
                {
                    return None;
                }
            }
            let metadata = std::fs::symlink_metadata(&canonical).ok()?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return None;
            }
            #[cfg(unix)]
            let key = source_key(&canonical);
            #[cfg(unix)]
            let signature = watcher_scan::file_signature(&metadata);
            #[cfg(unix)]
            if state.as_ref().is_some_and(|state| {
                state.files.get(&key) == Some(expected_hash)
                    && state.signatures.get(&key) == Some(&signature)
            }) {
                return Some(source.clone());
            }
            let current_hash = match rehash_bytes_remaining {
                Some(remaining) => {
                    let size = metadata.len();
                    if size > remaining {
                        return None;
                    }
                    let capacity = usize::try_from(size).ok()?;
                    let mut bytes = Vec::with_capacity(capacity);
                    let read = std::fs::File::open(&canonical)
                        .ok()?
                        .take(remaining.saturating_add(1))
                        .read_to_end(&mut bytes)
                        .ok()?;
                    let read_bytes = read as u64;
                    rehash_bytes_remaining = Some(remaining.saturating_sub(read_bytes));
                    if read_bytes != size || read_bytes > remaining {
                        return None;
                    }
                    maestria_core::content_hash(&bytes)
                }
                None => maestria_core::content_hash(&std::fs::read(canonical).ok()?),
            };
            (current_hash.as_str() == expected_hash.as_str()).then(|| source.clone())
        })
        .collect()
}
