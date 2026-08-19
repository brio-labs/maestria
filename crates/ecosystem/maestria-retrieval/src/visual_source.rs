use maestria_domain::{Evidence, EvidenceKind, IndexStatus, verify_snapshot_bytes};
use maestria_governance::{RetrievalDecision, scan_secrets};
use maestria_ports::VisualSource;

use super::VisualReranker;
use crate::types::RetrievalError;

/// Maximum number of bytes accepted for a visual source snapshot.
pub(super) const MAX_VISUAL_SOURCE_BYTES: usize = 8 * 1024 * 1024;

impl VisualReranker {
    /// Converts evidence into the visual source descriptor used for embedding.
    pub(super) fn visual_source(evidence: &Evidence) -> Option<VisualSource> {
        match &evidence.kind {
            EvidenceKind::PdfSpan {
                snapshot,
                page_start,
                page_end,
            } => Some(VisualSource::Page {
                blob: snapshot.blob_id(),
                page_start: *page_start,
                page_end: *page_end,
            }),
            EvidenceKind::PdfRegion {
                snapshot,
                page,
                x,
                y,
                width,
                height,
            } => Some(VisualSource::Region {
                blob: snapshot.blob_id(),
                page: *page,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            }),
            _ => None,
        }
    }

    /// Loads the blob backing the evidence and returns it with its visual source.
    pub(super) fn source_bytes(
        &self,
        evidence: &Evidence,
    ) -> Result<(VisualSource, Vec<u8>), RetrievalError> {
        let Some(source) = Self::visual_source(evidence) else {
            return Err(RetrievalError::Internal(
                "visual reranker candidate has no PDF source".to_string(),
            ));
        };
        let Some(artifact) = self
            .parts
            .artifacts
            .get(evidence.artifact_id)
            .map_err(|error| RetrievalError::Internal(error.to_string()))?
        else {
            return Err(RetrievalError::Internal(
                "visual reranker artifact is missing".to_string(),
            ));
        };
        // Precompute the authorization once instead of re-deriving the scope
        // intersection per security check.
        let authorization = self
            .parts
            .policy
            .authorization_context(&maestria_domain::CorpusScope::Global)
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        if artifact.index_status != IndexStatus::Indexed
            || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Err(RetrievalError::Internal(
                "visual reranker candidate failed security checks".to_string(),
            ));
        }
        let snapshot = match &evidence.kind {
            EvidenceKind::PdfSpan { snapshot, .. } | EvidenceKind::PdfRegion { snapshot, .. } => {
                snapshot
            }
            _ => {
                return Err(RetrievalError::Internal(
                    "visual reranker candidate has no PDF snapshot".to_string(),
                ));
            }
        };
        if evidence.artifact_id != artifact.id {
            return Err(RetrievalError::Internal(format!(
                "visual reranker evidence {} belongs to artifact {}, expected {}",
                evidence.id, evidence.artifact_id, artifact.id
            )));
        }
        if artifact.content_hash.as_ref() != Some(snapshot.content_hash()) {
            return Err(RetrievalError::Internal(format!(
                "visual reranker evidence {} source snapshot hash does not match owning artifact: expected {:?}, got {}",
                evidence.id,
                artifact.content_hash,
                snapshot.content_hash().as_str()
            )));
        }
        let bytes = self
            .parts
            .blobs
            .get(snapshot.blob_id())
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        verify_snapshot_bytes(snapshot, &bytes).map_err(|error| {
            RetrievalError::Internal(format!(
                "visual reranker evidence {} source snapshot verification failed: {}",
                evidence.id, error
            ))
        })?;
        if bytes.len() > MAX_VISUAL_SOURCE_BYTES
            || !scan_secrets(&String::from_utf8_lossy(&bytes)).is_clean()
        {
            return Err(RetrievalError::Internal(
                "visual reranker source failed privacy checks".to_string(),
            ));
        }
        Ok((source, bytes))
    }
}
