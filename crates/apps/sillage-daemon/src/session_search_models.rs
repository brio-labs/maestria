use std::{path::PathBuf, sync::Arc};

use sillage_domain::{
    ArtifactId, ArtifactVersionId, ContentHash, EvidenceId, LineRange, SearchOutcome, SearchPlan,
    SourceChoice,
};

use super::{control::SessionControl, resources::SessionResourceState};
/// Counts observed while preparing exactly the selected source scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCoverage {
    pub selected_roots: u32,
    pub visited_entries: u32,
    pub indexed_sources: u32,
    pub unsupported_sources: u32,
    pub excluded_sources: u32,
    pub oversized_sources: u32,
    pub failed_sources: u32,
    pub deferred_entries: u32,
    pub needs_ocr_sources: u32,
    pub bytes_examined: u64,
    pub coverage_complete: bool,
}

impl SessionCoverage {
    pub(super) fn new() -> Self {
        Self {
            selected_roots: 1,
            visited_entries: 0,
            indexed_sources: 0,
            unsupported_sources: 0,
            excluded_sources: 0,
            oversized_sources: 0,
            failed_sources: 0,
            deferred_entries: 0,
            needs_ocr_sources: 0,
            bytes_examined: 0,
            coverage_complete: true,
        }
    }
}

/// A current preview reopened from the selected Markdown source.
#[derive(Debug, Clone)]
pub struct SessionPreview {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub artifact_version: ArtifactVersionId,
    pub source_path: PathBuf,
    pub content_hash: ContentHash,
    pub line_range: LineRange,
    pub source_revision: i64,
    pub excerpt: String,
    pub truncated: bool,
}

/// Search outcome, current previews, and selected-scope coverage for one request.
#[derive(Debug, Clone)]
pub struct SessionSearchResponse {
    pub plan: SearchPlan,
    pub outcome: SearchOutcome,
    pub previews: Vec<SessionPreview>,
    pub coverage: SessionCoverage,
    pub source_revision: i64,
    pub stale_or_denied_candidates: u32,
}

/// An owned in-process retrieval session over one explicitly selected Markdown file.
pub struct SessionSearchRuntime {
    pub(super) resources: Arc<SessionResourceState>,
    pub(super) choice: SourceChoice,
    pub(super) selected_path: PathBuf,
    pub(super) source_root: PathBuf,
    pub(super) manifest: sillage_core::InstanceManifest,
    pub(super) coverage: SessionCoverage,
    pub(super) control: Arc<SessionControl>,
}
