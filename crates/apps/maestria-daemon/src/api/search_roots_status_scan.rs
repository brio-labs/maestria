use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use crate::api::protocol::{SearchExcludedSource, SearchRootStatus};
use maestria_core::InstanceManifest;

use super::inventory::IndexedInventory;

pub(super) fn root_inventory(
    manifest: &InstanceManifest,
    roots: &[PathBuf],
    mut indexed: IndexedInventory,
    roots_truncated: bool,
) -> (Vec<SearchRootStatus>, bool, Vec<SearchExcludedSource>, bool) {
    let mut remaining_budget = super::MAX_EXCLUSION_SCAN_ENTRIES;
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
        let (path, path_truncated) = super::bounded_display(root);
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
            super::inventory::push_excluded_sample(
                samples,
                samples_truncated,
                root,
                entry.path(),
                reason,
            );
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

pub(super) fn privacy_exclusions(manifest: &InstanceManifest) -> (Vec<String>, bool) {
    let mut truncated = manifest.excluded_patterns.len() > super::MAX_STATUS_PRIVACY_PATTERNS;
    let patterns = manifest
        .excluded_patterns
        .iter()
        .take(super::MAX_STATUS_PRIVACY_PATTERNS)
        .map(|pattern| {
            let (pattern, pattern_truncated) =
                super::bounded_text(pattern, super::MAX_STATUS_PATH_BYTES);
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
