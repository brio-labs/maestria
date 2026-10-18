use std::fmt::Write;
use std::path::Path;

use sillage_daemon::{
    SessionCoverage, SessionOperationSettlement, SessionPreview, SessionSearchResponse,
    SessionSearchRuntime,
};
use sillage_domain::{EvidenceCandidate, FreshnessStatus, SearchStatus, SourceLocation};

use super::state::{SessionSourceState, SourcePhase, SourceQuery};
use crate::application::passages::{
    Passage, PassageLocation, PassageSearchResult, ReopenedPassage, SearchMetadata,
    SessionEvidenceBinding,
};

pub(super) struct MappedSearch {
    pub(super) passages: PassageSearchResult,
    pub(super) phase: SourcePhase,
    pub(super) status: String,
    pub(super) detail: String,
    pub(super) source_revision: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SourceOperationFailure {
    Worker(String),
    RoutineSettlement(String),
}

impl SourceOperationFailure {
    pub(super) fn from_operation_message(
        message: String,
        settlement: Option<SessionOperationSettlement>,
    ) -> Self {
        match settlement {
            Some(_) => Self::RoutineSettlement(message),
            None => Self::Worker(message),
        }
    }

    pub(super) fn message(&self) -> &str {
        match self {
            Self::Worker(message) | Self::RoutineSettlement(message) => message,
        }
    }

    pub(super) fn is_routine_settlement(&self) -> bool {
        matches!(self, Self::RoutineSettlement(_))
    }
}

impl From<String> for SourceOperationFailure {
    fn from(message: String) -> Self {
        Self::Worker(message)
    }
}

pub(super) async fn search(
    runtime: &SessionSearchRuntime,
    selected_path: &Path,
    request: &SourceQuery,
    state: &SessionSourceState,
) -> Result<Option<MappedSearch>, SourceOperationFailure> {
    if !state.query_is_current(request) {
        return Ok(None);
    }
    let limit = runtime.choice().budget().max_results() as usize;
    let response = runtime
        .search_cancellable(request.query.clone(), limit, request.cancellation.clone())
        .await
        .map_err(|error| {
            let settlement = SessionOperationSettlement::classify_error(&error);
            let message = format!("{error:#}");
            SourceOperationFailure::from_operation_message(message, settlement)
        })?;
    if !state.query_is_current(request) {
        return Ok(None);
    }
    map_response(runtime, selected_path, request, response, limit)
        .await
        .map(Some)
        .map_err(SourceOperationFailure::Worker)
}

pub(super) async fn map_response(
    runtime: &SessionSearchRuntime,
    selected_path: &Path,
    request: &SourceQuery,
    response: SessionSearchResponse,
    limit: usize,
) -> Result<MappedSearch, String> {
    let SessionSearchResponse {
        outcome,
        previews,
        coverage,
        source_revision,
        stale_or_denied_candidates,
        ..
    } = response;
    let status_code = outcome.status.clone();
    let index_generation = outcome.index_generation;
    let preview_count = previews.len();
    let mut previews_by_id: Vec<SessionPreview> = Vec::with_capacity(preview_count);
    for preview in previews {
        // Later duplicate IDs replace earlier previews, matching map collection semantics.
        if let Some(existing_index) = previews_by_id
            .iter()
            .position(|existing| existing.evidence_id == preview.evidence_id)
        {
            previews_by_id[existing_index] = preview;
        } else {
            previews_by_id.push(preview);
        }
    }
    let mut hidden = stale_or_denied_candidates;
    let mut passages = Vec::with_capacity(limit.min(preview_count));
    let Some(path_text) = selected_path.to_str() else {
        return Err("The authorized source path is no longer representable".to_string());
    };
    for candidate in outcome.evidence.into_iter().take(limit) {
        let Some(passage) = map_candidate_to_passage(
            candidate,
            &mut previews_by_id,
            selected_path,
            path_text,
            request,
            source_revision,
        ) else {
            hidden = hidden.saturating_add(1);
            continue;
        };
        passages.push(passage);
    }
    if runtime
        .source_revision()
        .map_err(|error| format!("{error:#}"))?
        != source_revision
    {
        return Err(
            "The selected source changed during search; its results were withdrawn. Search again."
                .to_string(),
        );
    }
    let phase = phase_for_status(&status_code, coverage.coverage_complete, hidden);
    let status = result_status(&status_code, hidden);
    let detail = coverage_detail(
        &coverage,
        &status_code,
        index_generation.value(),
        source_revision,
        hidden,
    );
    let metadata = SearchMetadata::for_on_demand(status.clone(), detail.clone());
    Ok(MappedSearch {
        passages: PassageSearchResult {
            passages,
            paths: Vec::new(),
            metadata,
        },
        phase,
        status,
        detail,
        source_revision,
    })
}

fn map_candidate_to_passage(
    candidate: EvidenceCandidate,
    previews: &mut Vec<SessionPreview>,
    selected_path: &Path,
    path_text: &str,
    request: &SourceQuery,
    source_revision: i64,
) -> Option<Passage> {
    let freshness = candidate.freshness();
    if freshness == FreshnessStatus::Stale {
        return None;
    }
    let (candidate_path, start_line, end_line) = match candidate.source_span().location() {
        SourceLocation::File {
            path,
            start_line,
            end_line,
        } => (path.as_str(), *start_line, *end_line),
        _ => return None,
    };
    if !same_selected_path(Path::new(candidate_path), selected_path) {
        return None;
    }
    let preview_index = previews
        .iter()
        .position(|preview| preview.evidence_id == candidate.evidence_id())?;
    let preview = previews.remove(preview_index);
    if preview.artifact_version != candidate.artifact_version()
        || preview.source_revision != source_revision
        || preview.line_range.start() != start_line as usize
        || preview.line_range.end() != end_line as usize
    {
        return None;
    }
    if !same_selected_path(&preview.source_path, selected_path) {
        return None;
    }
    let binding = SessionEvidenceBinding {
        session_epoch: request.epoch,
        query_generation: request.generation,
        request_id: request.request_id,
        source_revision,
        evidence_id: candidate.evidence_id(),
        artifact_id: preview.artifact_id,
        artifact_version: candidate.artifact_version(),
        content_hash: preview.content_hash.clone(),
        line_range: preview.line_range,
        source_path: selected_path.to_path_buf(),
    };
    Some(Passage {
        evidence_id: candidate.evidence_id().value(),
        artifact_version: candidate.artifact_version().value(),
        excerpt: preview.excerpt,
        truncated: preview.truncated,
        location: PassageLocation::File {
            path: path_text.to_owned(),
            start_line,
            end_line,
            content_hash: binding.content_hash.as_str().to_owned(),
        },
        session_binding: Some(binding),
    })
}

pub(super) async fn reopen_current(
    runtime: &SessionSearchRuntime,
    binding: &SessionEvidenceBinding,
    cancellation: &sillage_daemon::SessionSearchCancellation,
) -> Result<ReopenedPassage, SourceOperationFailure> {
    let preview = runtime
        .preview_evidence(
            binding.evidence_id,
            binding.artifact_version,
            binding.source_revision,
            cancellation.clone(),
        )
        .await
        .map_err(|error| {
            let settlement = SessionOperationSettlement::classify_error(&error);
            let message = format!("{error:#}");
            SourceOperationFailure::from_operation_message(message, settlement)
        })?;
    verify_preview(&preview, binding).map_err(SourceOperationFailure::Worker)?;
    let current_revision = runtime
        .source_revision()
        .map_err(|error| SourceOperationFailure::Worker(format!("{error:#}")))?;
    if current_revision != binding.source_revision {
        return Err(SourceOperationFailure::Worker(
            "The selected source revision changed; the previous excerpt is no longer current."
                .to_string(),
        ));
    }
    let path = binding.source_path.to_str().ok_or_else(|| {
        SourceOperationFailure::Worker(
            "The authorized source path is no longer representable".to_string(),
        )
    })?;
    let (start, end) = (binding.line_range.start(), binding.line_range.end());
    let citation = if start == end {
        format!("{path}:{start}")
    } else {
        format!("{path}:{start}–{end}")
    };
    Ok(ReopenedPassage {
        excerpt: preview.excerpt,
        citation,
        path: Some(binding.source_path.clone()),
        pdf_page: None,
        fallback: "Evidence was reopened against the current authorized source revision."
            .to_string(),
    })
}

fn verify_preview(
    preview: &SessionPreview,
    binding: &SessionEvidenceBinding,
) -> Result<(), String> {
    if preview.evidence_id != binding.evidence_id
        || preview.artifact_id != binding.artifact_id
        || preview.artifact_version != binding.artifact_version
        || preview.source_revision != binding.source_revision
        || preview.content_hash != binding.content_hash
        || preview.line_range != binding.line_range
        || !same_selected_path(&preview.source_path, &binding.source_path)
    {
        return Err(
            "The source identity, version, locator, or revision changed; stale evidence was refused."
                .to_string(),
        );
    }
    Ok(())
}

fn same_selected_path(candidate: &Path, selected: &Path) -> bool {
    candidate == selected
}

fn phase_for_status(status: &SearchStatus, complete: bool, hidden: u32) -> SourcePhase {
    match status {
        SearchStatus::DeniedByPolicy
        | SearchStatus::QuarantinedForReview
        | SearchStatus::StaleEvidenceOnly => SourcePhase::Refused,
        SearchStatus::EvidenceIncomplete | SearchStatus::AnswerableWithWarnings => {
            SourcePhase::Partial
        }
        _ if !complete || hidden != 0 => SourcePhase::Partial,
        _ => SourcePhase::Results,
    }
}

fn result_status(status: &SearchStatus, hidden: u32) -> String {
    let lead = match status {
        SearchStatus::Answerable => "Current evidence found within the selected file",
        SearchStatus::AnswerableWithWarnings => {
            "Evidence found with retrieval or coverage warnings"
        }
        SearchStatus::EvidenceIncomplete => "Evidence is incomplete within the selected file",
        SearchStatus::SourcesConflict => {
            "Conflicting evidence was reported within the selected file"
        }
        SearchStatus::StaleEvidenceOnly => "No current evidence is available",
        SearchStatus::NoEvidenceFound => "No evidence was found in the examined selected file",
        SearchStatus::Abstained => {
            "Retrieval did not produce enough evidence within the selected file"
        }
        SearchStatus::DeniedByPolicy => "Current source policy refused this search",
        SearchStatus::QuarantinedForReview => "Evidence is unavailable pending review",
    };
    format!(
        "{lead}; {hidden} stale, changed, or denied candidate(s) were withdrawn. A scoped zero-hit result is not a whole-machine absence claim."
    )
}

fn coverage_detail(
    coverage: &SessionCoverage,
    status: &SearchStatus,
    index_generation: u64,
    source_revision: i64,
    hidden: u32,
) -> String {
    let extent = if coverage.coverage_complete {
        "Examination reported complete within the selected file"
    } else {
        "Partial examination; conclusions apply only to examined data in the selected file"
    };
    let mut detail = format!(
        "{extent}: {} selected root(s), {} entries visited, {} sources read, {} excluded, {} unsupported, {} need OCR (not run), {} oversized, {} failed, {} deferred, {} bytes examined.\n",
        coverage.selected_roots,
        coverage.visited_entries,
        coverage.indexed_sources,
        coverage.excluded_sources,
        coverage.unsupported_sources,
        coverage.needs_ocr_sources,
        coverage.oversized_sources,
        coverage.failed_sources,
        coverage.deferred_entries,
        coverage.bytes_examined,
    );
    let _ = writeln!(
        detail,
        "Retrieval: {status:?}; revision {source_revision}; index generation {index_generation}; {hidden} candidate(s) withdrawn.",
    );
    detail.push_str("Representation: lexical retrieval from the exact selected Markdown file. No model inference, model download, or sibling-file scope. Logical admission is not an RSS or physical-memory cap.");
    detail
}
