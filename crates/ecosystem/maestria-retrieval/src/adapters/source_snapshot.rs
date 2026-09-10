use std::sync::Arc;

use maestria_domain::{Evidence, EvidenceKind, verify_snapshot_bytes, verify_text_snapshot};
use maestria_ports::BlobStore;

use crate::types::RetrievalError;

/// Verifies immutable source snapshots before a candidate crosses retrieval.
#[derive(Clone)]
pub struct SourceSnapshotVerifier {
    blobs: Arc<dyn BlobStore + Send + Sync>,
}

impl SourceSnapshotVerifier {
    pub fn new(blobs: Arc<dyn BlobStore + Send + Sync>) -> Self {
        Self { blobs }
    }

    pub fn verify(
        &self,
        evidence: &Evidence,
        artifact: &maestria_domain::Artifact,
    ) -> Result<(), RetrievalError> {
        match &evidence.kind {
            EvidenceKind::FileSpan {
                snapshot, range, ..
            } => {
                if evidence.artifact_id != artifact.id {
                    return Err(RetrievalError::Internal(format!(
                        "evidence {} belongs to artifact {}, expected owning artifact {}",
                        evidence.id, evidence.artifact_id, artifact.id
                    )));
                }
                if artifact.content_hash.as_ref() != Some(snapshot.content_hash()) {
                    return Err(RetrievalError::Internal(format!(
                        "evidence {} source snapshot hash does not match owning artifact: expected {:?}, got {}",
                        evidence.id,
                        artifact.content_hash,
                        snapshot.content_hash().as_str()
                    )));
                }
                let bytes = self
                    .blobs
                    .get(snapshot.blob_id())
                    .map_err(super::common::port_error)?;
                verify_text_snapshot(snapshot, &bytes, Some(range), &evidence.excerpt).map_err(
                    |error| {
                        RetrievalError::Internal(format!(
                            "evidence {} source snapshot verification failed: {}",
                            evidence.id, error
                        ))
                    },
                )
            }
            EvidenceKind::WebSnapshot { snapshot, .. } => {
                if evidence.artifact_id != artifact.id {
                    return Err(RetrievalError::Internal(format!(
                        "evidence {} belongs to artifact {}, expected owning artifact {}",
                        evidence.id, evidence.artifact_id, artifact.id
                    )));
                }
                if artifact.content_hash.as_ref() != Some(snapshot.content_hash()) {
                    return Err(RetrievalError::Internal(format!(
                        "evidence {} source snapshot hash does not match owning artifact: expected {:?}, got {}",
                        evidence.id,
                        artifact.content_hash,
                        snapshot.content_hash().as_str()
                    )));
                }
                let bytes = self
                    .blobs
                    .get(snapshot.blob_id())
                    .map_err(super::common::port_error)?;
                verify_text_snapshot(snapshot, &bytes, None, &evidence.excerpt).map_err(|error| {
                    RetrievalError::Internal(format!(
                        "evidence {} source snapshot verification failed: {}",
                        evidence.id, error
                    ))
                })
            }
            EvidenceKind::PdfSpan { snapshot, .. } | EvidenceKind::PdfRegion { snapshot, .. } => {
                if evidence.artifact_id != artifact.id {
                    return Err(RetrievalError::Internal(format!(
                        "evidence {} belongs to artifact {}, expected owning artifact {}",
                        evidence.id, evidence.artifact_id, artifact.id
                    )));
                }
                if artifact.content_hash.as_ref() != Some(snapshot.content_hash()) {
                    return Err(RetrievalError::Internal(format!(
                        "evidence {} source snapshot hash does not match owning artifact: expected {:?}, got {}",
                        evidence.id,
                        artifact.content_hash,
                        snapshot.content_hash().as_str()
                    )));
                }
                let bytes = self
                    .blobs
                    .get(snapshot.blob_id())
                    .map_err(super::common::port_error)?;
                verify_snapshot_bytes(snapshot, &bytes).map_err(|error| {
                    RetrievalError::Internal(format!(
                        "evidence {} source snapshot verification failed: {}",
                        evidence.id, error
                    ))
                })
            }
            EvidenceKind::CommandOutput { .. }
            | EvidenceKind::TestResult { .. }
            | EvidenceKind::Diff { .. }
            | EvidenceKind::Validation { .. } => Ok(()),
        }
    }
    /// Verifies a source snapshot without ever allocating beyond `max_bytes`.
    ///
    /// The bound is applied to the blob read before hashing or UTF-8
    /// validation; a snapshot larger than the bound fails closed.
    pub fn verify_bounded(
        &self,
        evidence: &Evidence,
        artifact: &maestria_domain::Artifact,
        max_bytes: usize,
    ) -> Result<Vec<u8>, RetrievalError> {
        if max_bytes == 0 {
            return Err(RetrievalError::Internal(
                "source snapshot bound must be positive".to_string(),
            ));
        }
        if evidence.artifact_id != artifact.id {
            return Err(RetrievalError::Internal(
                "evidence does not belong to owning artifact".to_string(),
            ));
        }
        let (snapshot, range) = match &evidence.kind {
            EvidenceKind::FileSpan {
                snapshot, range, ..
            } => (snapshot, Some(range)),
            EvidenceKind::WebSnapshot { snapshot, .. } => (snapshot, None),
            _ => {
                return Err(RetrievalError::Internal(
                    "evidence does not have a bounded text snapshot".to_string(),
                ));
            }
        };
        if artifact.content_hash.as_ref() != Some(snapshot.content_hash()) {
            return Err(RetrievalError::Internal(
                "source snapshot hash does not match owning artifact".to_string(),
            ));
        }
        let bytes = self
            .blobs
            .get_bounded(snapshot.blob_id(), max_bytes)
            .map_err(super::common::port_error)?;
        verify_text_snapshot(snapshot, &bytes, range, &evidence.excerpt).map_err(|error| {
            RetrievalError::Internal(format!(
                "bounded source snapshot verification failed: {error}"
            ))
        })?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "source_snapshot_tests.rs"]
mod tests;
