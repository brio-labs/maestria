use maestria_domain::{ArtifactId, EvidenceKind, IndexStatus, SourceSpan};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EmbeddingProvenance,
    EvidenceRepository, RetentionPolicy, VectorEmbedding, VectorIndex, VisualEmbeddingProvider,
    VisualEmbeddingRequest, VisualSource,
};

use super::common::port_error;
use super::visual::{VisualGenerationCapability, ensure_local_no_retention};
use crate::types::{RetrievalError, RetrievalResult};

/// Dependencies for rebuilding a governed visual page/region projection.
pub struct VisualProjectionRebuildParts<'a> {
    pub index: &'a dyn VectorIndex,
    pub artifacts: &'a dyn ArtifactRepository,
    pub chunks: &'a dyn ChunkRepository,
    pub evidence: &'a dyn EvidenceRepository,
    pub blobs: &'a dyn BlobStore,
    pub policy: &'a RetrievalSecurityPolicy,
    pub provider: &'a dyn VisualEmbeddingProvider,
}

/// Rebuilds a separate visual projection for the supplied active artifacts.
///
/// The caller must supply an active `visual_page_v1` identity. The projection
/// never shares storage rows with dense text embeddings and applies the same
/// retrieval policy and secret gates as the visual query lane.
pub fn rebuild_visual_projection(
    parts: VisualProjectionRebuildParts<'_>,
    artifact_ids: &[ArtifactId],
    capability: &VisualGenerationCapability,
) -> RetrievalResult<()> {
    if parts.provider.identity() != Some(capability.identity().clone()) {
        return Err(RetrievalError::Internal(
            "visual provider identity does not match active generation capability".to_string(),
        ));
    }
    ensure_local_no_retention(parts.provider)?;
    let mut embeddings = Vec::new();
    for artifact_id in artifact_ids {
        let Some(artifact) = parts.artifacts.get(*artifact_id).map_err(port_error)? else {
            continue;
        };
        if artifact.index_status != IndexStatus::Indexed
            || parts.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
        {
            continue;
        }
        for chunk in parts
            .chunks
            .list_for_artifact(*artifact_id)
            .map_err(port_error)?
        {
            if let Some(embedding) = visual_embedding_for_chunk(
                &chunk,
                parts.evidence,
                parts.blobs,
                parts.policy,
                parts.provider,
                capability.identity(),
            )? {
                embeddings.push(embedding);
            }
        }
    }
    parts.index.rebuild(embeddings).map_err(port_error)
}

fn visual_embedding_for_chunk(
    chunk: &maestria_domain::Chunk,
    evidence: &dyn EvidenceRepository,
    blobs: &dyn BlobStore,
    policy: &RetrievalSecurityPolicy,
    provider: &dyn VisualEmbeddingProvider,
    identity: &EmbeddingIdentity,
) -> RetrievalResult<Option<VectorEmbedding>> {
    if !matches!(
        &chunk.source_span,
        SourceSpan::PdfSpan { .. } | SourceSpan::PdfRegion { .. }
    ) || !scan_secrets(&chunk.text).is_clean()
    {
        return Ok(None);
    }
    let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
    let Some(record) = evidence.get(evidence_id).map_err(port_error)? else {
        return Ok(None);
    };
    if policy.evaluate(&record.security) != RetrievalDecision::Allowed
        || !scan_secrets(&record.excerpt).is_clean()
    {
        return Ok(None);
    }
    let Some(source) = visual_source_for_evidence(&record.kind) else {
        return Ok(None);
    };
    let blob = match &record.kind {
        EvidenceKind::PdfSpan { blob, .. } | EvidenceKind::PdfRegion { blob, .. } => *blob,
        _ => return Ok(None),
    };
    let bytes = blobs.get(blob).map_err(port_error)?;
    if bytes.is_empty() || !scan_secrets(&String::from_utf8_lossy(&bytes)).is_clean() {
        return Ok(None);
    }
    let response = provider
        .embed_source(VisualEmbeddingRequest {
            source,
            bytes: bytes.clone(),
            identity: identity.clone(),
        })
        .map_err(port_error)?;
    if response.identity != *identity {
        return Err(RetrievalError::Internal(
            "visual source response identity changed during projection rebuild".to_string(),
        ));
    }
    if response.disclosure.remote || response.disclosure.retention != RetentionPolicy::NoRetention {
        return Err(RetrievalError::Internal(
            "visual provider violates local no-retention policy".to_string(),
        ));
    }
    Ok(Some(VectorEmbedding {
        chunk_id: chunk.id,
        vector: response.vector,
        provenance: EmbeddingProvenance {
            content_hash: maestria_domain::content_hash(&bytes),
            identity: response.identity,
            provider_id: response.provider_id,
            model: response.model,
            model_version: response.model_version,
            disclosure: response.disclosure,
        },
    }))
}

fn visual_source_for_evidence(kind: &EvidenceKind) -> Option<VisualSource> {
    match kind {
        EvidenceKind::PdfSpan {
            blob,
            page_start,
            page_end,
        } => Some(VisualSource::Page {
            blob: *blob,
            page_start: *page_start,
            page_end: *page_end,
        }),
        EvidenceKind::PdfRegion {
            blob,
            page,
            x,
            y,
            width,
            height,
        } => Some(VisualSource::Region {
            blob: *blob,
            page: *page,
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        }),
        _ => None,
    }
}
