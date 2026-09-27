use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use maestria_core::InstanceManifest;

#[derive(Default)]
pub(super) struct IndexedInventory {
    pub(super) sources: BTreeMap<PathBuf, Vec<String>>,
    pub(super) indexed_counts: BTreeMap<PathBuf, usize>,
    pub(super) stale_counts: BTreeMap<PathBuf, usize>,
    pub(super) path_truncated: BTreeMap<PathBuf, bool>,
    pub(super) excluded_sources: Vec<crate::api::protocol::SearchExcludedSource>,
    pub(super) ocr_needed_counts: BTreeMap<PathBuf, usize>,
    pub(super) excluded_sources_truncated: bool,
}

pub(super) fn current_ocr_needed_sources(
    manifest: &InstanceManifest,
    roots: &[PathBuf],
    indexed_sources: &[(String, String)],
    fresh_sources: &BTreeSet<String>,
    artifact_ids: &BTreeMap<String, u64>,
    state: &maestria_domain::KernelState,
) -> BTreeSet<String> {
    indexed_sources
        .iter()
        .filter_map(|(source, source_hash)| {
            if !fresh_sources.contains(source)
                || !Path::new(source)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            {
                return None;
            }
            let source_path = Path::new(source);
            if !manifest.allows_source(source_path)
                || maestria_index_selection::is_privacy_excluded_path(source_path)
            {
                return None;
            }
            let root = roots
                .iter()
                .filter(|root| source_path.starts_with(root))
                .max_by_key(|root| root.components().count())?;
            if crate::watcher::is_internal_source_path(manifest, root, source_path) {
                return None;
            }
            let artifact_id = maestria_domain::ArtifactId::new(*artifact_ids.get(source)?);
            let artifact = state.artifacts.get(&artifact_id)?;
            if artifact
                .content_hash
                .as_ref()
                .is_none_or(|hash| hash.as_str() != source_hash)
                || artifact.parse_status != Some(maestria_domain::ParseStatus::NeedsOcr)
            {
                return None;
            }
            Some(source.clone())
        })
        .collect()
}

pub(super) fn indexed_inventory(
    manifest: &InstanceManifest,
    roots: &[PathBuf],
    indexed_sources: &[(String, String)],
    fresh_sources: &BTreeSet<String>,
    ocr_needed_sources: &BTreeSet<String>,
) -> IndexedInventory {
    let mut inventory = IndexedInventory::default();
    for root in roots {
        inventory.sources.insert(root.clone(), Vec::new());
    }
    let mut remaining_samples = super::MAX_STATUS_SOURCE_PATHS;
    for (source, _) in indexed_sources {
        let source_path = Path::new(source);
        if !manifest.allows_source(source_path)
            || maestria_index_selection::is_privacy_excluded_path(source_path)
        {
            continue;
        }
        let Some(root) = manifest
            .read_roots
            .iter()
            .filter(|root| source_path.starts_with(root))
            .max_by_key(|root| root.components().count())
        else {
            continue;
        };
        if crate::watcher::is_internal_source_path(manifest, root, source_path)
            || !inventory.sources.contains_key(root)
        {
            continue;
        }
        if fresh_sources.contains(source) && ocr_needed_sources.contains(source) {
            *inventory
                .ocr_needed_counts
                .entry(root.to_path_buf())
                .or_default() += 1;
            push_excluded_sample(
                &mut inventory.excluded_sources,
                &mut inventory.excluded_sources_truncated,
                root,
                source_path,
                "needs_ocr",
            );
            continue;
        }
        if fresh_sources.contains(source) {
            *inventory
                .indexed_counts
                .entry(root.to_path_buf())
                .or_default() += 1;
            if remaining_samples > 0
                && let Some(paths) = inventory.sources.get_mut(root)
            {
                let (display, truncated) = super::bounded_display(source_path);
                paths.push(display);
                *inventory
                    .path_truncated
                    .entry(root.to_path_buf())
                    .or_default() |= truncated;
                remaining_samples -= 1;
            }
        } else {
            *inventory
                .stale_counts
                .entry(root.to_path_buf())
                .or_default() += 1;
            push_excluded_sample(
                &mut inventory.excluded_sources,
                &mut inventory.excluded_sources_truncated,
                root,
                source_path,
                "source_changed_or_missing",
            );
        }
    }
    inventory
}

pub(super) fn push_excluded_sample(
    samples: &mut Vec<crate::api::protocol::SearchExcludedSource>,
    samples_truncated: &mut bool,
    root: &Path,
    path: &Path,
    reason: &str,
) {
    if samples.len() >= super::MAX_EXCLUDED_SOURCE_SAMPLES {
        *samples_truncated = true;
        return;
    }
    let (root, _) = super::bounded_display(root);
    let (path, path_truncated) = super::bounded_display(path);
    samples.push(crate::api::protocol::SearchExcludedSource {
        root,
        path,
        path_truncated,
        reason: reason.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: [(u64, &str, &str, &str, bool); 5] = [
        (
            1,
            "/approved/scanned.pdf",
            "current image-only pdf",
            "current image-only pdf",
            true,
        ),
        (
            2,
            "/approved/stale-snapshot.pdf",
            "current bytes",
            "different stored bytes",
            true,
        ),
        (
            3,
            "/outside/scanned.pdf",
            "outside bytes",
            "outside bytes",
            true,
        ),
        (
            4,
            "/approved/document.md",
            "markdown bytes",
            "markdown bytes",
            true,
        ),
        (
            5,
            "/approved/stale.pdf",
            "stale bytes",
            "stale bytes",
            false,
        ),
    ];

    #[test]
    fn ocr_needed_sources_require_fresh_approved_pdf_and_current_snapshot() -> anyhow::Result<()> {
        let root = PathBuf::from("/approved");
        let manifest =
            InstanceManifest::default_for_root(root.clone(), maestria_test_support::realm_id(10)?);
        let mut state = maestria_domain::KernelState::default();
        let mut indexed_sources = Vec::new();
        let mut fresh_sources = BTreeSet::new();
        let mut artifact_ids = BTreeMap::new();
        for (id, path, source_bytes, artifact_bytes, fresh) in CASES {
            let artifact_id = maestria_domain::ArtifactId::new(id);
            let source = path.to_string();
            let source_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(
                source_bytes.as_bytes(),
            ))?;
            let artifact_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(
                artifact_bytes.as_bytes(),
            ))?;
            indexed_sources.push((source.clone(), source_hash.as_str().to_owned()));
            artifact_ids.insert(source.clone(), id);
            if fresh {
                fresh_sources.insert(source);
            }
            std::sync::Arc::make_mut(&mut state.artifacts).insert(
                artifact_id,
                maestria_domain::Artifact {
                    id: artifact_id,
                    title: path.to_string(),
                    chunk_ids: BTreeSet::new(),
                    card_ids: BTreeSet::new(),
                    claim_ids: BTreeSet::new(),
                    evidence_ids: BTreeSet::new(),
                    index_status: maestria_domain::IndexStatus::Indexed,
                    content_hash: Some(artifact_hash),
                    parse_status: Some(maestria_domain::ParseStatus::NeedsOcr),
                    security: maestria_domain::SecurityMetadata::default(),
                },
            );
        }

        let ocr_needed = current_ocr_needed_sources(
            &manifest,
            std::slice::from_ref(&root),
            &indexed_sources,
            &fresh_sources,
            &artifact_ids,
            &state,
        );

        assert_eq!(
            ocr_needed,
            BTreeSet::from(["/approved/scanned.pdf".to_string()])
        );

        let inventory = indexed_inventory(
            &manifest,
            std::slice::from_ref(&root),
            &[(
                "/approved/scanned.pdf".to_string(),
                maestria_domain::content_hash(b"current image-only pdf"),
            )],
            &fresh_sources,
            &ocr_needed,
        );
        assert!(!inventory.indexed_counts.contains_key(&root));
        assert_eq!(inventory.ocr_needed_counts.get(&root), Some(&1));
        assert_eq!(inventory.excluded_sources.len(), 1);
        assert_eq!(inventory.excluded_sources[0].reason, "needs_ocr");
        Ok(())
    }
}
