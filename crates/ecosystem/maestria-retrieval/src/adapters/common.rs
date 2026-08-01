use std::{collections::BTreeSet, sync::Arc};

use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};
use async_trait::async_trait;
use maestria_domain::{
    ArtifactVersionId, ContentRange, Evidence, EvidenceCandidate, EvidenceKind, EvidenceSpan,
    FreshnessStatus, IndexGenerationId, RetrievalReason, RetrievalScoreSet, SearchLaneStatus,
    SourceLocation, SourceSpan, StructureNodeId, TrustLabel,
};

pub(super) fn port_error(error: maestria_ports::PortError) -> RetrievalError {
    RetrievalError::Internal(error.to_string())
}

pub(super) fn one_based_rank(rank: usize) -> u32 {
    match u32::try_from(rank.saturating_add(1)) {
        Ok(rank) => rank,
        Err(e) => {
            let _ = e;
            u32::MAX
        }
    }
}

pub(super) fn generation_mismatch(
    expected: IndexGenerationId,
    actual: IndexGenerationId,
) -> RetrievalError {
    RetrievalError::Internal(format!(
        "retriever generation mismatch: expected {expected}, found {actual}"
    ))
}

/// Filters retriever results to the latest known artifact version per source.
pub struct CurrentVersionFilter {
    inner: Arc<dyn CandidateRetriever>,
    active_versions: Arc<BTreeSet<ArtifactVersionId>>,
}

impl CurrentVersionFilter {
    pub fn new(
        inner: Arc<dyn CandidateRetriever>,
        active_versions: BTreeSet<ArtifactVersionId>,
    ) -> Self {
        Self {
            inner,
            active_versions: Arc::new(active_versions),
        }
    }
}

#[async_trait]
impl CandidateRetriever for CurrentVersionFilter {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.inner.descriptor()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        let mut batch = self.inner.retrieve(request).await?;
        if self.active_versions.is_empty() {
            batch.candidates.clear();
            batch.status = SearchLaneStatus::Empty;
        } else {
            batch
                .candidates
                .retain(|candidate| self.active_versions.contains(&candidate.artifact_version));
            if batch.candidates.is_empty() && matches!(batch.status, SearchLaneStatus::Succeeded) {
                batch.status = SearchLaneStatus::Empty;
            }
        }
        Ok(batch)
    }
}

pub(super) fn candidate_from_records(
    artifact_id: maestria_domain::ArtifactId,
    source_span: &SourceSpan,
    evidence: &Evidence,
    node_id: StructureNodeId,
    scores: RetrievalScoreSet,
    reasons: Vec<RetrievalReason>,
) -> Result<EvidenceCandidate, RetrievalError> {
    if evidence.artifact_id != artifact_id {
        return Err(RetrievalError::Internal(format!(
            "candidate evidence {} belongs to artifact {}, expected {}",
            evidence.id, evidence.artifact_id, artifact_id
        )));
    }
    let (location, range) = evidence_location(evidence, source_span)?;
    let source_span = EvidenceSpan::new(Some(node_id), location, range)
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
    Ok(EvidenceCandidate {
        evidence_id: evidence.id,
        artifact_version: ArtifactVersionId::new(artifact_id.value()),
        source_span,
        scores,
        trust: TrustLabel::Unverified,
        freshness: FreshnessStatus::Unknown,
        duplicate_cluster: None,
        reasons,
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
                SourceSpan::PdfRegion { .. } => {
                    return Err(RetrievalError::Internal(
                        "file evidence has a PDF region source span".to_string(),
                    ));
                }
            };
            Ok((
                SourceLocation::file(path.clone(), start_line, end_line)?,
                ContentRange::new(range.start(), range.end())
                    .map_err(|error| RetrievalError::Internal(error.to_string()))?,
            ))
        }
        EvidenceKind::PdfSpan {
            page_start,
            page_end,
            ..
        } => Ok((
            SourceLocation::page(*page_start, *page_end)?,
            ContentRange::new(0, 1).map_err(|error| RetrievalError::Internal(error.to_string()))?,
        )),
        EvidenceKind::PdfRegion {
            page,
            x,
            y,
            width,
            height,
            ..
        } => Ok((
            SourceLocation::region(*page, *x, *y, *width, *height)?,
            ContentRange::new(0, 1).map_err(|error| RetrievalError::Internal(error.to_string()))?,
        )),
        EvidenceKind::WebSnapshot { url, .. } => Ok((
            SourceLocation::symbol(url.clone(), "web_snapshot".to_string())?,
            ContentRange::new(0, 1).map_err(|error| RetrievalError::Internal(error.to_string()))?,
        )),
        _ => Ok((
            SourceLocation::symbol(format!("evidence:{}", evidence.id), "evidence".to_string())?,
            ContentRange::new(0, 1).map_err(|error| RetrievalError::Internal(error.to_string()))?,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_construction_rejects_cross_artifact_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact_id = maestria_domain::ArtifactId::new(1);
        let mut evidence = Evidence {
            id: maestria_domain::EvidenceId::new(1),
            artifact_id: maestria_domain::ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: maestria_domain::ValidationReportId::new(1),
            },
            excerpt: "alpha".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: Default::default(),
        };
        evidence.artifact_id = maestria_domain::ArtifactId::new(2);
        let result = candidate_from_records(
            artifact_id,
            &SourceSpan::text_span(1, 1).map_err(|e| RetrievalError::Internal(e.to_string()))?,
            &evidence,
            StructureNodeId::new(1),
            RetrievalScoreSet::empty(),
            Vec::new(),
        );
        assert!(matches!(
            result,
            Err(RetrievalError::Internal(message))
                if message.contains("belongs to artifact")
        ));
        Ok(())
    }
}
