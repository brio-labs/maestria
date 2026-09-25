use std::{path::PathBuf, sync::Arc};

use maestria_core::InstanceLayout;
use maestria_domain::{
    ActiveSourceVersions, ArtifactVersionId, EvidenceCandidate, EvidenceKind, FreshnessStatus,
    SourceLocation,
};
use maestria_governance::RetrievalAuthorizationContext;

use super::protocol::{
    EvidenceSourceResponse, FederationSearchResponse, SearchPassagePreviewResponse,
};

pub(super) const RESPONSE_EVIDENCE_RESERVE_BYTES: usize = 4 * 1024;

const SEARCH_PATH_RESULTS_FIELD_PREFIX: &[u8] = b",\"path_results\":[";

pub(super) fn fit_search_path_results(response: &mut FederationSearchResponse) {
    if response.search.path_results.is_empty() {
        return;
    }
    let max_response_bytes =
        super::MAX_REQUEST_BYTES.saturating_sub(RESPONSE_EVIDENCE_RESERVE_BYTES + 1);
    let path_results = std::mem::take(&mut response.search.path_results);
    let Ok(base) = serde_json::to_vec(response) else {
        return;
    };
    let mut response_bytes = base.len();
    let mut fitted = Vec::with_capacity(path_results.len());
    for result in path_results {
        let Ok(encoded) = serde_json::to_vec(&result) else {
            break;
        };
        let separator_bytes = if fitted.is_empty() {
            SEARCH_PATH_RESULTS_FIELD_PREFIX.len() + 1
        } else {
            1
        };
        let added_bytes = encoded.len().saturating_add(separator_bytes);
        if response_bytes.saturating_add(added_bytes) > max_response_bytes {
            continue;
        }
        response_bytes += added_bytes;
        fitted.push(result);
    }
    response.search.path_results = fitted;
}
const SEARCH_PREVIEW_FIELD_PREFIX: &[u8] = b",\"preview\":";

#[derive(Debug)]
pub(super) struct SearchPreviewCandidate {
    evidence_index: usize,
    evidence_id: u64,
    artifact_version: ArtifactVersionId,
    location: SourceLocation,
}

pub(super) fn search_preview_candidates(
    evidence: &[EvidenceCandidate],
) -> Vec<SearchPreviewCandidate> {
    evidence
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| search_preview_candidate(index, candidate))
        .collect()
}

fn search_preview_candidate(
    evidence_index: usize,
    candidate: &EvidenceCandidate,
) -> Option<SearchPreviewCandidate> {
    // Index candidates can carry Unknown when the retrieval lane has no
    // freshness signal; the scoped re-open independently verifies currentness.
    // A known-stale candidate is never previewed.
    if matches!(candidate.freshness(), FreshnessStatus::Stale) {
        return None;
    }
    Some(SearchPreviewCandidate {
        evidence_index,
        evidence_id: candidate.evidence_id().value(),
        artifact_version: candidate.artifact_version(),
        location: candidate.source_span().location().clone(),
    })
}

pub(super) async fn open_and_attach_search_previews(
    layout: &InstanceLayout,
    response: &mut FederationSearchResponse,
    candidates: Vec<SearchPreviewCandidate>,
    authorization: RetrievalAuthorizationContext,
    max_evidence_bytes: usize,
    current_sources: Option<(i64, Arc<ActiveSourceVersions>)>,
    allowed_roots: Option<Arc<[PathBuf]>>,
) {
    if candidates.is_empty() {
        return;
    }
    let evidence_ids: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.evidence_id)
        .collect();
    let layout = layout.clone();
    let opened = tokio::task::spawn_blocking(move || {
        crate::evidence_open::open_evidence_scoped_batch_with_authorization(
            &layout,
            &evidence_ids,
            &authorization,
            current_sources
                .as_ref()
                .map(|(revision, sources)| (*revision, sources.as_ref())),
            allowed_roots.as_deref(),
        )
    })
    .await;
    if let Ok(Ok(opened)) = opened {
        attach_search_previews(response, candidates, opened, max_evidence_bytes);
    }
}

fn attach_search_previews(
    response: &mut FederationSearchResponse,
    candidates: Vec<SearchPreviewCandidate>,
    opened: Vec<anyhow::Result<maestria_core::OpenEvidenceOutput>>,
    max_evidence_bytes: usize,
) {
    let max_response_bytes =
        super::MAX_REQUEST_BYTES.saturating_sub(RESPONSE_EVIDENCE_RESERVE_BYTES + 1);
    let Ok(serialized_response) = serde_json::to_vec(response) else {
        return;
    };
    let mut response_bytes = serialized_response.len();
    if response_bytes > max_response_bytes {
        return;
    }

    let max_excerpt_bytes = max_evidence_bytes
        .min(super::MAX_REQUEST_BYTES.saturating_sub(RESPONSE_EVIDENCE_RESERVE_BYTES));
    let count = candidates.len().min(opened.len());
    let mut remaining = count;
    for (candidate, opened) in candidates.into_iter().zip(opened).take(count) {
        remaining -= 1;
        let Some(search_evidence) = response.search.evidence.get_mut(candidate.evidence_index)
        else {
            continue;
        };
        if search_evidence.evidence_id != candidate.evidence_id {
            continue;
        }
        let Ok(opened) = opened else {
            continue;
        };
        let Some(location) = matching_preview_source(&candidate, &opened) else {
            continue;
        };
        let available_bytes = max_response_bytes.saturating_sub(response_bytes);
        let candidate_budget = available_bytes / (remaining + 1);
        let Some(preview) = bounded_search_preview(
            &opened.evidence.excerpt,
            location,
            max_excerpt_bytes,
            candidate_budget,
        ) else {
            continue;
        };
        let increment = search_preview_increment(&preview);
        if response_bytes.saturating_add(increment) > max_response_bytes {
            continue;
        }
        response_bytes += increment;
        search_evidence.preview = Some(preview);
    }
}

fn matching_preview_source(
    candidate: &SearchPreviewCandidate,
    opened: &maestria_core::OpenEvidenceOutput,
) -> Option<EvidenceSourceResponse> {
    let evidence = &opened.evidence;
    if evidence.id.value() != candidate.evidence_id || evidence.artifact_id != opened.artifact.id {
        return None;
    }
    match (&candidate.location, &evidence.kind) {
        (
            SourceLocation::File {
                path: candidate_path,
                start_line,
                end_line,
            },
            EvidenceKind::FileSpan {
                path,
                range,
                snapshot,
            },
        ) if candidate_path == path
            && u32::try_from(range.start()).ok() == Some(*start_line)
            && u32::try_from(range.end()).ok() == Some(*end_line)
            && preview_snapshot_matches(candidate, &opened.artifact, snapshot) =>
        {
            Some(EvidenceSourceResponse::File {
                path: path.clone(),
                start_line: *start_line,
                end_line: *end_line,
                content_hash: snapshot.content_hash().as_str().to_owned(),
            })
        }
        (
            SourceLocation::DocxParagraph {
                path: candidate_path,
                start_paragraph: candidate_start,
                end_paragraph: candidate_end,
            },
            EvidenceKind::DocxParagraphSpan {
                path,
                range,
                snapshot,
            },
        ) if candidate_path == path
            && range.start() == *candidate_start
            && range.end() == *candidate_end
            && preview_snapshot_matches(candidate, &opened.artifact, snapshot) =>
        {
            Some(EvidenceSourceResponse::DocxParagraph {
                path: path.clone(),
                start_paragraph: *candidate_start,
                end_paragraph: *candidate_end,
                content_hash: snapshot.content_hash().as_str().to_owned(),
            })
        }
        _ => matching_pdf_preview_source(candidate, opened),
    }
}

fn matching_pdf_preview_source(
    candidate: &SearchPreviewCandidate,
    opened: &maestria_core::OpenEvidenceOutput,
) -> Option<EvidenceSourceResponse> {
    match (&candidate.location, &opened.evidence.kind) {
        (
            SourceLocation::Page {
                page_start: candidate_start,
                page_end: candidate_end,
            },
            EvidenceKind::PdfSpan {
                snapshot,
                page_start,
                page_end,
            },
        ) if candidate_start == page_start
            && candidate_end == page_end
            && preview_snapshot_matches(candidate, &opened.artifact, snapshot) =>
        {
            Some(EvidenceSourceResponse::Pdf {
                snapshot_id: snapshot.blob_id().value(),
                page_start: *page_start,
                page_end: *page_end,
                path: None,
            })
        }
        (
            SourceLocation::Region {
                page: candidate_page,
                x: candidate_x,
                y: candidate_y,
                width: candidate_width,
                height: candidate_height,
            },
            EvidenceKind::PdfRegion {
                snapshot,
                page,
                x,
                y,
                width,
                height,
            },
        ) if candidate_page == page
            && candidate_x == x
            && candidate_y == y
            && candidate_width == width
            && candidate_height == height
            && preview_snapshot_matches(candidate, &opened.artifact, snapshot) =>
        {
            Some(EvidenceSourceResponse::PdfRegion {
                snapshot_id: snapshot.blob_id().value(),
                page: *page,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                path: None,
            })
        }
        _ => None,
    }
}

fn preview_snapshot_matches(
    candidate: &SearchPreviewCandidate,
    artifact: &maestria_domain::Artifact,
    snapshot: &maestria_domain::SnapshotRef,
) -> bool {
    artifact.content_hash.as_ref() == Some(snapshot.content_hash())
        && snapshot.content_hash().version_id().ok() == Some(candidate.artifact_version)
}

fn bounded_search_preview(
    excerpt: &str,
    location: EvidenceSourceResponse,
    max_excerpt_bytes: usize,
    max_response_delta: usize,
) -> Option<SearchPassagePreviewResponse> {
    if excerpt.is_empty() || max_excerpt_bytes == 0 {
        return None;
    }
    let mut low = 0;
    let mut high = excerpt.len().min(max_excerpt_bytes);
    while low < high {
        let midpoint = low + (high - low).div_ceil(2);
        let bounded = maestria_ports::truncate_at_char_boundary(excerpt, midpoint);
        let preview = SearchPassagePreviewResponse {
            excerpt: bounded.to_owned(),
            truncated: bounded.len() < excerpt.len(),
            location: location.clone(),
        };
        let encoded_bytes = search_preview_increment(&preview);
        if encoded_bytes <= max_response_delta {
            low = midpoint;
        } else {
            high = midpoint - 1;
        }
    }
    let bounded = maestria_ports::truncate_at_char_boundary(excerpt, low);
    if bounded.is_empty() {
        return None;
    }
    let preview = SearchPassagePreviewResponse {
        excerpt: bounded.to_owned(),
        truncated: bounded.len() < excerpt.len(),
        location,
    };
    let encoded_bytes = search_preview_increment(&preview);
    (encoded_bytes <= max_response_delta).then_some(preview)
}

fn search_preview_increment(preview: &SearchPassagePreviewResponse) -> usize {
    match serde_json::to_vec(preview) {
        Ok(bytes) => bytes
            .len()
            .saturating_add(SEARCH_PREVIEW_FIELD_PREFIX.len()),
        Err(_) => usize::MAX,
    }
}

#[cfg(test)]
#[path = "federation_previews/tests.rs"]
mod tests;
