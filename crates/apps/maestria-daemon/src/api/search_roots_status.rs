use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use maestria_core::InstanceManifest;

use super::super::super::protocol::{
    SearchExcludedSource, SearchIndexingStatus, SearchRootStatus, SearchRootsStatusResponse,
};
use super::super::super::server::ApiContext;
use super::MAX_APPROVED_ROOTS;

const MAX_STATUS_SOURCE_PATHS: usize = 8;
const MAX_EXCLUDED_SOURCE_SAMPLES: usize = 8;
const MAX_EXCLUSION_SCAN_ENTRIES: usize = 20_000;
const MAX_STATUS_PATH_BYTES: usize = 256;
const MAX_STATUS_PRIVACY_PATTERNS: usize = 8;
const MAX_STATUS_ERROR_BYTES: usize = 512;

#[derive(Default)]
struct IndexedInventory {
    sources: BTreeMap<PathBuf, Vec<String>>,
    indexed_counts: BTreeMap<PathBuf, usize>,
    stale_counts: BTreeMap<PathBuf, usize>,
    path_truncated: BTreeMap<PathBuf, bool>,
    excluded_sources: Vec<SearchExcludedSource>,
    ocr_needed_counts: BTreeMap<PathBuf, usize>,
    excluded_sources_truncated: bool,
}

pub(in crate::api::services) async fn status(
    context: &ApiContext,
) -> anyhow::Result<SearchRootsStatusResponse> {
    status_for_roots(context, None).await
}

pub(in crate::api::services) async fn status_for_roots(
    context: &ApiContext,
    allowed_roots: Option<&[PathBuf]>,
) -> anyhow::Result<SearchRootsStatusResponse> {
    let mut manifest = context.source_manifest.read().clone();
    if let Some(roots) = allowed_roots {
        manifest.read_roots.retain(|root| roots.contains(root));
    }
    let approved_root_count = manifest.read_roots.len();
    let roots_truncated = approved_root_count > MAX_APPROVED_ROOTS;
    let status_roots = manifest
        .read_roots
        .iter()
        .take(MAX_APPROVED_ROOTS)
        .cloned()
        .collect::<Vec<_>>();
    let watcher = crate::watcher::status(&context.layout);
    let fresh_sources =
        crate::watcher::fresh_source_paths(&context.layout, &manifest, &watcher.indexed_sources);
    let runtime_state = match context.runtime.as_ref() {
        Some(runtime) => Some(runtime.kernel_state().await),
        None => None,
    };
    let ocr_needed_sources = runtime_state.as_ref().map_or_else(BTreeSet::new, |state| {
        current_ocr_needed_sources(
            &manifest,
            &manifest.read_roots,
            &watcher.indexed_sources,
            &fresh_sources,
            &watcher.current_artifact_ids,
            state,
        )
    });
    let indexed = indexed_inventory(
        &manifest,
        &status_roots,
        &watcher.indexed_sources,
        &fresh_sources,
        &ocr_needed_sources,
    );
    let (roots, exclusion_scan_truncated, excluded_sources, excluded_sources_truncated) =
        root_inventory(&manifest, &status_roots, indexed, roots_truncated);
    let (privacy_exclusions, privacy_exclusions_truncated) = privacy_exclusions(&manifest);
    let (last_error, last_error_truncated) =
        watcher
            .last_error
            .as_deref()
            .map_or((None, false), |error| {
                let (error, truncated) = bounded_text(error, MAX_STATUS_ERROR_BYTES);
                (Some(error), truncated)
            });
    Ok(SearchRootsStatusResponse {
        roots,
        approved_root_count,
        ocr_needed_file_count: ocr_needed_sources.len(),
        roots_truncated,
        supported_formats: maestria_index_selection::supported_source_formats()
            .iter()
            .map(|format| (*format).to_string())
            .collect(),
        ignored_by_default: vec![
            "hidden files and directories".to_string(),
            ".ignore rules".to_string(),
            ".gitignore rules".to_string(),
            "symbolic links (not followed)".to_string(),
            "Git global excludes (disabled)".to_string(),
        ],
        privacy_exclusions,
        privacy_exclusions_truncated,
        excluded_sources,
        excluded_sources_truncated,
        exclusion_scan_truncated,
        indexing: SearchIndexingStatus {
            scanning: watcher.scanning,
            pending_file_count: watcher.pending_files,
            last_scan_unix_ms: watcher.last_scan_unix_ms,
            last_error,
            last_error_truncated,
        },
    })
}

fn current_ocr_needed_sources(
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
fn indexed_inventory(
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
    let mut remaining_samples = MAX_STATUS_SOURCE_PATHS;
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
                let (display, truncated) = bounded_display(source_path);
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

fn root_inventory(
    manifest: &InstanceManifest,
    roots: &[PathBuf],
    mut indexed: IndexedInventory,
    roots_truncated: bool,
) -> (Vec<SearchRootStatus>, bool, Vec<SearchExcludedSource>, bool) {
    let mut remaining_budget = MAX_EXCLUSION_SCAN_ENTRIES;
    let mut scan_truncated = roots_truncated;
    let mut root_statuses = Vec::with_capacity(roots.len());
    for root in roots {
        let (selected, selected_truncated) = selected_files(manifest, root, &mut remaining_budget);
        let (excluded_count, mut exclusions_by_reason, root_scan_truncated) = excluded_inventory(
            manifest,
            root,
            &selected,
            !selected_truncated,
            &mut remaining_budget,
            &mut indexed.excluded_sources,
            &mut indexed.excluded_sources_truncated,
        );
        let mut stale_count = 0;
        if let Some(count) = indexed.stale_counts.get(root) {
            stale_count = *count;
        }
        if stale_count > 0 {
            *exclusions_by_reason
                .entry("source_changed_or_missing".to_string())
                .or_default() += stale_count;
        }
        let mut ocr_needed_count = 0;
        if let Some(count) = indexed.ocr_needed_counts.get(root) {
            ocr_needed_count = *count;
        }
        if ocr_needed_count > 0 {
            *exclusions_by_reason
                .entry("needs_ocr".to_string())
                .or_default() += ocr_needed_count;
        }
        let excluded_file_count = excluded_count + stale_count + ocr_needed_count;
        let root_scan_truncated = selected_truncated || root_scan_truncated;
        scan_truncated |= root_scan_truncated;
        let mut indexed_count = 0;
        if let Some(count) = indexed.indexed_counts.get(root) {
            indexed_count = *count;
        }
        let mut indexed_sources = Vec::new();
        if let Some(sources) = indexed.sources.remove(root) {
            indexed_sources = sources;
        }
        let (path, path_truncated) = bounded_display(root);
        root_statuses.push(SearchRootStatus {
            path,
            path_truncated,
            indexed_file_count: indexed_count,
            indexed_sources_truncated: indexed_sources.len() < indexed_count,
            indexed_sources,
            indexed_source_paths_truncated: indexed
                .path_truncated
                .get(root)
                .copied()
                .is_some_and(|truncated| truncated),
            excluded_file_count,
            exclusions_by_reason,
            exclusion_scan_truncated: root_scan_truncated,
        });
    }
    (
        root_statuses,
        scan_truncated,
        indexed.excluded_sources,
        indexed.excluded_sources_truncated,
    )
}

fn selected_files(
    manifest: &InstanceManifest,
    root: &Path,
    remaining_budget: &mut usize,
) -> (BTreeSet<PathBuf>, bool) {
    let limit = (*remaining_budget / 2).max(1);
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .sort_by_file_name(|left, right| left.cmp(right))
        .build();
    let mut selected = BTreeSet::new();
    let mut truncated = false;
    for (visited, result) in walker.enumerate() {
        if *remaining_budget == 0 || visited >= limit {
            truncated = true;
            break;
        }
        *remaining_budget -= 1;
        let Ok(entry) = result else {
            truncated = true;
            continue;
        };
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file()
            && !file_type.is_symlink()
            && manifest.allows_source(entry.path())
            && !maestria_index_selection::is_privacy_excluded_path(entry.path())
            && !crate::watcher::is_internal_source_path(manifest, root, entry.path())
            && maestria_index_selection::is_supported_source_file(entry.path())
        {
            selected.insert(entry.path().to_path_buf());
        }
    }
    (selected, truncated)
}

fn excluded_inventory(
    manifest: &InstanceManifest,
    root: &Path,
    selected: &BTreeSet<PathBuf>,
    selected_complete: bool,
    remaining_budget: &mut usize,
    samples: &mut Vec<SearchExcludedSource>,
    samples_truncated: &mut bool,
) -> (usize, BTreeMap<String, usize>, bool) {
    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .follow_links(false)
        .sort_by_file_name(|left, right| left.cmp(right))
        .build();
    let mut count = 0;
    let mut by_reason = BTreeMap::new();
    let mut truncated = false;
    for result in walker {
        if *remaining_budget == 0 {
            truncated = true;
            break;
        }
        *remaining_budget -= 1;
        let Ok(entry) = result else {
            truncated = true;
            continue;
        };
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let reason = exclusion_reason(
            manifest,
            root,
            selected,
            selected_complete,
            &entry,
            file_type,
        );
        if let Some(reason) = reason {
            count += 1;
            *by_reason.entry(reason.to_string()).or_insert(0) += 1;
            push_excluded_sample(samples, samples_truncated, root, entry.path(), reason);
        }
    }
    (count, by_reason, truncated)
}

fn exclusion_reason(
    manifest: &InstanceManifest,
    root: &Path,
    selected: &BTreeSet<PathBuf>,
    selected_complete: bool,
    entry: &ignore::DirEntry,
    file_type: std::fs::FileType,
) -> Option<&'static str> {
    if file_type.is_symlink() {
        Some("symbolic_link")
    } else if !file_type.is_file() {
        None
    } else if crate::watcher::is_internal_source_path(manifest, root, entry.path()) {
        Some("instance_internal")
    } else if !manifest.allows_source(entry.path())
        || maestria_index_selection::is_privacy_excluded_path(entry.path())
    {
        Some("privacy_exclusion")
    } else if is_hidden_path(root, entry.path()) {
        Some("hidden")
    } else if !maestria_index_selection::is_supported_source_file(entry.path()) {
        Some("unsupported_format")
    } else if !selected_complete {
        None
    } else if !selected.contains(entry.path()) {
        Some("ignore_rule")
    } else {
        None
    }
}

fn push_excluded_sample(
    samples: &mut Vec<SearchExcludedSource>,
    samples_truncated: &mut bool,
    root: &Path,
    path: &Path,
    reason: &str,
) {
    if samples.len() >= MAX_EXCLUDED_SOURCE_SAMPLES {
        *samples_truncated = true;
        return;
    }
    let (root, _) = bounded_display(root);
    let (path, path_truncated) = bounded_display(path);
    samples.push(SearchExcludedSource {
        root,
        path,
        path_truncated,
        reason: reason.to_string(),
    });
}

fn privacy_exclusions(manifest: &InstanceManifest) -> (Vec<String>, bool) {
    let mut truncated = manifest.excluded_patterns.len() > MAX_STATUS_PRIVACY_PATTERNS;
    let patterns = manifest
        .excluded_patterns
        .iter()
        .take(MAX_STATUS_PRIVACY_PATTERNS)
        .map(|pattern| {
            let (pattern, pattern_truncated) = bounded_text(pattern, MAX_STATUS_PATH_BYTES);
            truncated |= pattern_truncated;
            pattern
        })
        .collect();
    (patterns, truncated)
}

fn is_hidden_path(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative
            .components()
            .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
    })
}

fn bounded_display(path: &Path) -> (String, bool) {
    bounded_text(&path.display().to_string(), MAX_STATUS_PATH_BYTES)
}

fn bounded_text(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let limit = max_bytes.saturating_sub('…'.len_utf8());
    let mut boundary = 0;
    for (index, _) in text.char_indices().take_while(|(index, _)| *index <= limit) {
        boundary = index;
    }
    (format!("{}…", &text[..boundary]), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_needed_sources_require_fresh_approved_pdf_and_current_snapshot() -> anyhow::Result<()> {
        let root = PathBuf::from("/approved");
        let manifest =
            InstanceManifest::default_for_root(root.clone(), maestria_test_support::realm_id(10)?);
        let cases = [
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
        let mut state = maestria_domain::KernelState::default();
        let mut indexed_sources = Vec::new();
        let mut fresh_sources = BTreeSet::new();
        let mut artifact_ids = BTreeMap::new();
        for (id, path, source_bytes, artifact_bytes, fresh) in cases {
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
