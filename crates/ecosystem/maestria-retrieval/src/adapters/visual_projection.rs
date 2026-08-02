use maestria_domain::{ArtifactId, EvidenceKind, IndexStatus, SourceSpan, verify_snapshot_bytes};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EmbeddingProvenance,
    EvidenceRepository, ProviderDisclosure, VectorEmbedding, VectorIndex, VisualEmbeddingProvider,
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
    let disclosure = ensure_local_no_retention(parts.provider)?;
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
                &artifact,
                &parts,
                capability.identity(),
                &disclosure,
            )? {
                embeddings.push(embedding);
            }
        }
    }
    parts.index.rebuild(embeddings).map_err(port_error)
}

fn visual_embedding_for_chunk(
    chunk: &maestria_domain::Chunk,
    artifact: &maestria_domain::Artifact,
    parts: &VisualProjectionRebuildParts<'_>,
    identity: &EmbeddingIdentity,
    disclosure: &ProviderDisclosure,
) -> RetrievalResult<Option<VectorEmbedding>> {
    if !matches!(
        &chunk.source_span,
        SourceSpan::PdfSpan { .. } | SourceSpan::PdfRegion { .. }
    ) || !scan_secrets(&chunk.text).is_clean()
    {
        return Ok(None);
    }
    let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
    let Some(record) = parts.evidence.get(evidence_id).map_err(port_error)? else {
        return Ok(None);
    };
    if record.artifact_id != chunk.artifact_id {
        return Err(RetrievalError::Internal(format!(
            "visual evidence {} belongs to artifact {}, expected {}",
            record.id, record.artifact_id, chunk.artifact_id
        )));
    }
    if parts.policy.evaluate(&record.security) != RetrievalDecision::Allowed
        || !scan_secrets(&record.excerpt).is_clean()
    {
        return Ok(None);
    }
    let snapshot = match &record.kind {
        EvidenceKind::PdfSpan { snapshot, .. } | EvidenceKind::PdfRegion { snapshot, .. } => {
            snapshot
        }
        _ => return Ok(None),
    };
    if artifact.id != chunk.artifact_id || record.artifact_id != artifact.id {
        return Err(RetrievalError::Internal(format!(
            "visual evidence {} belongs to artifact {}, expected {}",
            record.id, record.artifact_id, artifact.id
        )));
    }
    if artifact.content_hash.as_deref() != Some(snapshot.content_hash().as_str()) {
        return Err(RetrievalError::Internal(format!(
            "visual evidence {} source snapshot hash does not match owning artifact: expected {:?}, got {}",
            record.id,
            artifact.content_hash,
            snapshot.content_hash().as_str()
        )));
    }
    let Some(source) = visual_source_for_evidence(&record.kind) else {
        return Ok(None);
    };
    let bytes = parts.blobs.get(snapshot.blob_id()).map_err(port_error)?;
    verify_snapshot_bytes(snapshot, &bytes).map_err(|error| {
        RetrievalError::Internal(format!(
            "visual evidence {} source snapshot verification failed: {}",
            record.id, error
        ))
    })?;
    if !scan_secrets(&String::from_utf8_lossy(&bytes)).is_clean() {
        return Ok(None);
    }
    let response = parts
        .provider
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
    if response.disclosure != *disclosure {
        return Err(RetrievalError::Internal(
            "visual provider response disclosure changed during projection rebuild".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_ports::{
        EmbeddingResponse, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
        InMemoryEvidenceRepository, InMemoryVectorIndex, PortError, RetentionPolicy,
    };

    struct UnusedVisualProvider;

    impl VisualEmbeddingProvider for UnusedVisualProvider {
        fn disclosure(&self) -> maestria_ports::ProviderDisclosure {
            maestria_ports::ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            }
        }

        fn embed_query(
            &self,
            _query: &str,
            _identity: EmbeddingIdentity,
        ) -> Result<EmbeddingResponse, PortError> {
            Err(PortError::Downstream {
                message: "visual provider must not be called".to_string(),
            })
        }

        fn embed_source(
            &self,
            _request: VisualEmbeddingRequest,
        ) -> Result<EmbeddingResponse, PortError> {
            Err(PortError::Downstream {
                message: "visual provider must not be called".to_string(),
            })
        }

        fn identity(&self) -> Option<EmbeddingIdentity> {
            None
        }
    }

    #[test]
    fn projection_rejects_cross_artifact_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let artifact_id = ArtifactId::new(1);
        let evidence = InMemoryEvidenceRepository::new();
        evidence.put(maestria_domain::Evidence {
            id: maestria_domain::evidence_id_for(artifact_id, 0),
            artifact_id: ArtifactId::new(2),
            claim_id: None,
            kind: EvidenceKind::PdfSpan {
                snapshot: maestria_domain::SnapshotRef::new(
                    maestria_domain::BlobId::new(9),
                    maestria_domain::ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
                ),
                page_start: 1,
                page_end: 1,
            },
            excerpt: "figure".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: Default::default(),
        })?;
        let chunk = maestria_domain::Chunk {
            id: maestria_domain::ChunkId::new(1),
            artifact_id,
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: SourceSpan::pdf_span(1)?,
            representations: Vec::new(),
            order: 0,
            text: "figure".to_string(),
        };
        let identity = maestria_ports::contract_tests::fixture_embedding_identity("visual", 1)?;
        let artifact = maestria_domain::Artifact {
            id: artifact_id,
            title: "test".to_string(),
            chunk_ids: Default::default(),
            card_ids: Default::default(),
            claim_ids: Default::default(),
            evidence_ids: Default::default(),
            index_status: IndexStatus::Indexed,
            content_hash: Some("sha256:".to_owned() + &"0".repeat(64)),
            parse_status: None,
            security: Default::default(),
        };
        let index = InMemoryVectorIndex::new();
        let artifacts = InMemoryArtifactRepository::new();
        let chunks = InMemoryChunkRepository::new();
        let blobs = InMemoryBlobStore::new();
        let policy = RetrievalSecurityPolicy::default();
        let provider = UnusedVisualProvider;
        let parts = VisualProjectionRebuildParts {
            index: &index,
            artifacts: &artifacts,
            chunks: &chunks,
            evidence: &evidence,
            blobs: &blobs,
            policy: &policy,
            provider: &provider,
        };
        let result = visual_embedding_for_chunk(
            &chunk,
            &artifact,
            &parts,
            &identity,
            &provider.disclosure(),
        );
        assert!(matches!(
            result,
            Err(RetrievalError::Internal(message))
                if message.contains("visual evidence") && message.contains("expected")
        ));
        Ok(())
    }
}
