use maestria_domain::{Artifact, Chunk, ChunkId, Evidence, EvidenceKind, SourceSpan};
use maestria_governance::{RetrievalAuthorizationContext, RetrievalDecision, scan_secrets};
use maestria_ports::{ArtifactRepository, ChunkRepository, EvidenceRepository, PortError};

use super::SourceSnapshotVerifier;
use super::chunk_access::load_authorized_chunk;
use super::common::port_error;
use crate::types::RetrievalError;

pub(super) type VisualPrescoreRecord = (Artifact, Chunk, Evidence);

pub(super) fn load_authorized_visual_record(
    chunks: &dyn ChunkRepository,
    artifacts: &dyn ArtifactRepository,
    evidence_repository: &dyn EvidenceRepository,
    verifier: &SourceSnapshotVerifier,
    chunk_id: ChunkId,
    authorization: &RetrievalAuthorizationContext,
) -> Result<Option<VisualPrescoreRecord>, RetrievalError> {
    let Some((artifact, chunk)) =
        load_authorized_chunk(chunks, artifacts, chunk_id, authorization).map_err(port_error)?
    else {
        return Ok(None);
    };
    let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
    let Some(evidence) = evidence_repository.get(evidence_id).map_err(port_error)? else {
        return Ok(None);
    };
    if evidence.artifact_id != chunk.artifact_id || evidence.artifact_id != artifact.id {
        return Err(port_error(PortError::Conflict {
            message: format!(
                "visual evidence {} belongs to artifact {}, expected {}",
                evidence.id, evidence.artifact_id, artifact.id
            ),
        }));
    }
    if authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
        || !scan_secrets(&evidence.excerpt).is_clean()
    {
        return Ok(None);
    }
    if !visual_pdf_prerequisites(&chunk.source_span, &evidence.kind)
        || !visual_snapshot_matches_artifact(&evidence.kind, &artifact)
    {
        return Ok(None);
    }
    verifier.verify(&evidence, &artifact)?;
    Ok(Some((artifact, chunk, evidence)))
}

pub(super) fn visual_pdf_prerequisites(source_span: &SourceSpan, kind: &EvidenceKind) -> bool {
    match (source_span, kind) {
        (
            SourceSpan::PdfSpan { page },
            EvidenceKind::PdfSpan {
                page_start,
                page_end,
                ..
            },
        ) => {
            let Ok(page) = u32::try_from(*page) else {
                return false;
            };
            page > 0
                && *page_start > 0
                && *page_start <= *page_end
                && page >= *page_start
                && page <= *page_end
        }
        (
            SourceSpan::PdfRegion {
                page,
                x,
                y,
                width,
                height,
            },
            EvidenceKind::PdfRegion {
                page: evidence_page,
                x: evidence_x,
                y: evidence_y,
                width: evidence_width,
                height: evidence_height,
                ..
            },
        ) => {
            let Ok(page) = u32::try_from(*page) else {
                return false;
            };
            page > 0
                && *width > 0
                && *height > 0
                && page == *evidence_page
                && *x == *evidence_x
                && *y == *evidence_y
                && *width == *evidence_width
                && *height == *evidence_height
        }
        _ => false,
    }
}

fn visual_snapshot_matches_artifact(kind: &EvidenceKind, artifact: &Artifact) -> bool {
    let snapshot = match kind {
        EvidenceKind::PdfSpan { snapshot, .. } | EvidenceKind::PdfRegion { snapshot, .. } => {
            snapshot
        }
        _ => return false,
    };
    artifact.content_hash.as_deref() == Some(snapshot.content_hash().as_str())
}
