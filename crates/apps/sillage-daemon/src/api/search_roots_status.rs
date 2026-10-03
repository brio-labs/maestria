use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::super::super::protocol::{SearchIndexingStatus, SearchRootsStatusResponse};
use super::super::super::server::ApiContext;
use super::MAX_APPROVED_ROOTS;

#[path = "search_roots_status_inventory.rs"]
mod inventory;
#[path = "search_roots_status_scan.rs"]
mod scan;

use inventory::{current_ocr_needed_sources, indexed_inventory};
use scan::{privacy_exclusions, root_inventory};

const MAX_STATUS_SOURCE_PATHS: usize = 8;
const MAX_EXCLUDED_SOURCE_SAMPLES: usize = 8;
const MAX_EXCLUSION_SCAN_ENTRIES: usize = 20_000;
const MAX_STATUS_PATH_BYTES: usize = 256;
const MAX_STATUS_PRIVACY_PATTERNS: usize = 8;
const MAX_STATUS_ERROR_BYTES: usize = 512;

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
        supported_formats: sillage_index_selection::supported_source_formats()
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
