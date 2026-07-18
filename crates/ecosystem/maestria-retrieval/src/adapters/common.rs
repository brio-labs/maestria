use std::sync::Arc;

use maestria_domain::{
    ArtifactVersionId, ContentRange, Evidence, EvidenceCandidate, EvidenceKind, EvidenceSpan,
    FreshnessStatus, IndexGenerationId, RetrievalReason, RetrievalScoreSet, SourceLocation,
    SourceSpan, StructureNodeId, TrustLabel,
};
use maestria_ports::BlobStore;

use crate::types::RetrievalError;

pub(super) fn port_error(error: maestria_ports::PortError) -> RetrievalError {
    RetrievalError::Internal(error.to_string())
}

pub(super) fn generation_mismatch(
    expected: IndexGenerationId,
    actual: IndexGenerationId,
) -> RetrievalError {
    RetrievalError::Internal(format!(
        "retriever generation mismatch: expected {expected}, found {actual}"
    ))
}

/// Verifies immutable source snapshots before a candidate crosses retrieval.
pub struct SourceSnapshotVerifier {
    blobs: Arc<dyn BlobStore + Send + Sync>,
}

impl SourceSnapshotVerifier {
    pub fn new(blobs: Arc<dyn BlobStore + Send + Sync>) -> Self {
        Self { blobs }
    }

    pub fn verify(&self, evidence: &Evidence) -> Result<(), RetrievalError> {
        let (snapshot, expected_hash, excerpt) = match &evidence.kind {
            EvidenceKind::WebSnapshot {
                snapshot,
                content_hash,
                ..
            }
            | EvidenceKind::FileSpan {
                snapshot: Some(snapshot),
                content_hash,
                ..
            } => (
                Some(*snapshot),
                Some(content_hash),
                evidence.excerpt.as_str(),
            ),
            _ => (None, None, ""),
        };
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let bytes = self.blobs.get(snapshot).map_err(port_error)?;
        let actual_hash = maestria_domain::content_hash(&bytes);
        if expected_hash.is_some_and(|expected| expected != &actual_hash) {
            return Err(RetrievalError::Internal(format!(
                "evidence {} source snapshot hash mismatch",
                evidence.id
            )));
        }
        if !excerpt.is_empty()
            && !contains_compact_excerpt(&String::from_utf8_lossy(&bytes), excerpt)
        {
            return Err(RetrievalError::Internal(format!(
                "evidence {} excerpt is absent from its source snapshot",
                evidence.id
            )));
        }
        Ok(())
    }
}

fn contains_compact_excerpt(source: &str, excerpt: &str) -> bool {
    let needle: Vec<_> = excerpt.split_whitespace().collect();
    if needle.is_empty() {
        return true;
    }
    let mut remaining = source.split_whitespace();
    loop {
        let mut candidate = remaining.clone();
        if needle
            .iter()
            .all(|expected| candidate.next() == Some(*expected))
        {
            return true;
        }
        if remaining.next().is_none() {
            return false;
        }
    }
}
pub(super) fn candidate_from_records(
    artifact_id: maestria_domain::ArtifactId,
    source_span: &SourceSpan,
    evidence: &Evidence,
    node_id: StructureNodeId,
    score: u32,
) -> Result<EvidenceCandidate, RetrievalError> {
    let (location, range) = evidence_location(evidence, source_span)?;
    let source_span = EvidenceSpan::new(Some(node_id), location, range)
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
    Ok(EvidenceCandidate {
        evidence_id: evidence.id,
        artifact_version: ArtifactVersionId::new(artifact_id.value()),
        source_span,
        scores: RetrievalScoreSet {
            bm25: score,
            semantic_similarity: 0,
        },
        trust: TrustLabel::Unverified,
        freshness: FreshnessStatus::Unknown,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: Vec::new(),
    })
}

fn evidence_location(
    evidence: &Evidence,
    source_span: &SourceSpan,
) -> Result<(SourceLocation, ContentRange), RetrievalError> {
    match &evidence.kind {
        EvidenceKind::FileSpan { path, range, .. } => {
            let (start_line, end_line) = match source_span {
                SourceSpan::TextSpan {
                    start_line,
                    end_line,
                } => (*start_line as u32, *end_line as u32),
                SourceSpan::PdfSpan { .. } => {
                    return Err(RetrievalError::Internal(
                        "file evidence has a PDF source span".to_string(),
                    ));
                }
            };
            Ok((
                SourceLocation::File {
                    path: path.clone(),
                    start_line,
                    end_line,
                },
                *range,
            ))
        }
        EvidenceKind::PdfSpan {
            page_start,
            page_end,
            ..
        } => Ok((
            SourceLocation::Page {
                page_start: *page_start,
                page_end: *page_end,
            },
            ContentRange { start: 0, end: 1 },
        )),
        EvidenceKind::WebSnapshot { url, .. } => Ok((
            SourceLocation::Symbol {
                path: url.clone(),
                qualified_name: "web_snapshot".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        )),
        _ => Ok((
            SourceLocation::Symbol {
                path: format!("evidence:{}", evidence.id),
                qualified_name: "evidence".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        )),
    }
}
