use std::sync::Arc;

use maestria_domain::{Evidence, EvidenceKind, verify_snapshot_bytes, verify_text_snapshot};
use maestria_ports::BlobStore;

use crate::types::RetrievalError;

/// Verifies immutable source snapshots before a candidate crosses retrieval.
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
                if artifact.content_hash.as_deref() != Some(snapshot.content_hash().as_str()) {
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
                if artifact.content_hash.as_deref() != Some(snapshot.content_hash().as_str()) {
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
                if artifact.content_hash.as_deref() != Some(snapshot.content_hash().as_str()) {
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
}

#[cfg(test)]
#[path = "source_snapshot_tests.rs"]
mod tests;
