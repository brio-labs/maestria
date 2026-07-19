use std::sync::Arc;

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, port_error,
};
use crate::traits::CandidateRetriever;
use crate::types::{
    CandidateBatch, CandidateRequest, RetrievalError, RetrievalResult, RetrieverDescriptor,
};
use async_trait::async_trait;
use maestria_domain::{
    ArtifactId, CorpusSnapshotId, EvidenceCandidate, EvidenceKind, IndexGenerationId,
    IndexGenerationRegistry, IndexStatus, RepresentationName, SearchLaneStatus, SourceSpan,
};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EmbeddingProvenance,
    EvidenceRepository, RetentionPolicy, VectorEmbedding, VectorIndex, VectorSearchQuery,
    VisualEmbeddingProvider, VisualEmbeddingRequest, VisualSource,
};

/// Dependencies for the optional page/region visual retrieval lane.
pub struct VisualPageRegionRetrieverParts {
    pub index: Arc<dyn VectorIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
    pub embedding_provider: Arc<dyn VisualEmbeddingProvider + Send + Sync>,
}

/// Capability proving a visual provider is bound to an active current generation.
#[derive(Clone)]
pub struct VisualGenerationCapability {
    identity: EmbeddingIdentity,
}

impl VisualGenerationCapability {
    /// Validates representation, fingerprint, lifecycle, activation, and snapshot.
    pub fn activate(
        registry: &IndexGenerationRegistry,
        identity: EmbeddingIdentity,
        corpus_snapshot: CorpusSnapshotId,
    ) -> Result<Self, RetrievalError> {
        let name = RepresentationName::new("visual_page_v1");
        if identity.representation != name {
            return Err(RetrievalError::Internal(
                "visual provider representation must be visual_page_v1".to_string(),
            ));
        }
        let valid = registry
            .get(identity.generation_id)
            .is_some_and(|generation| {
                generation.name == name
                    && generation.corpus_snapshot == corpus_snapshot
                    && generation.fingerprint == identity.fingerprint
                    && registry.is_serveable(generation.id)
            });
        if !valid {
            return Err(RetrievalError::Internal(
                "visual provider identity does not match an active current visual_page_v1 generation"
                    .to_string(),
            ));
        }
        Ok(Self { identity })
    }

    /// Returns the validated active generation.
    pub fn generation(&self) -> IndexGenerationId {
        self.identity.generation_id
    }

    fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }
}

/// Retrieves only visual PDF chunks from a named visual generation.
///
/// The lane is injectable: text-only providers cannot be presented as visual.
pub struct VisualPageRegionRetriever {
    index: Arc<dyn VectorIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    embedding_provider: Arc<dyn VisualEmbeddingProvider + Send + Sync>,
    expected_identity: EmbeddingIdentity,
    verifier: SourceSnapshotVerifier,
    policy: RetrievalSecurityPolicy,
    descriptor: RetrieverDescriptor,
}

impl VisualPageRegionRetriever {
    pub fn new(
        parts: VisualPageRegionRetrieverParts,
        policy: RetrievalSecurityPolicy,
        capability: VisualGenerationCapability,
    ) -> Self {
        let expected_identity = capability.identity().clone();
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            embedding_provider: parts.embedding_provider,
            expected_identity: expected_identity.clone(),
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            policy,
            descriptor: RetrieverDescriptor {
                id: "visual_page_regions".to_string(),
                modality: "image".to_string(),
                representation: expected_identity.representation.clone(),
                generation: expected_identity.generation_id,
            },
        }
    }

    fn candidate_from_hit(
        &self,
        hit: maestria_ports::VectorSearchHit,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let Some(chunk) = self.chunks.get(hit.chunk_id).map_err(port_error)? else {
            return Ok(None);
        };
        if !matches!(
            &chunk.source_span,
            SourceSpan::PdfSpan { .. } | SourceSpan::PdfRegion { .. }
        ) {
            return Ok(None);
        }
        let Some(artifact) = self.artifacts.get(chunk.artifact_id).map_err(port_error)? else {
            return Ok(None);
        };
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
            return Ok(None);
        };
        if !matches!(
            evidence.kind,
            EvidenceKind::PdfSpan { .. } | EvidenceKind::PdfRegion { .. }
        ) || artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || self.policy.evaluate(&evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&chunk.text).is_clean()
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        self.verifier.verify(&evidence)?;
        let score = if hit.score.is_finite() && hit.score > 0.0 {
            (hit.score.min(1.0) * 1_000_000.0).floor() as u32
        } else {
            0
        };
        candidate_from_records(
            artifact.id,
            &chunk.source_span,
            &evidence,
            chunk.node_id,
            score,
        )
        .map(Some)
    }

    fn retrieve_with_vector(
        &self,
        vector: VectorSearchQuery,
        request: CandidateRequest,
    ) -> Result<CandidateBatch, RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        let candidates = self
            .index
            .search_similar(vector)
            .map_err(port_error)?
            .into_iter()
            .take(request.query.limit)
            .filter_map(|hit| self.candidate_from_hit(hit).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        let status = if candidates.is_empty() {
            SearchLaneStatus::Empty
        } else {
            SearchLaneStatus::Succeeded
        };
        Ok(CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates,
            status,
            generation: Some(self.descriptor.generation),
            bytes_read: 0,
        })
    }
}

fn ensure_local_no_retention(provider: &dyn VisualEmbeddingProvider) -> Result<(), RetrievalError> {
    let Some(disclosure) = provider.disclosure() else {
        return Err(RetrievalError::Internal(
            "visual provider disclosure is unavailable".to_string(),
        ));
    };
    if disclosure.remote || disclosure.retention != RetentionPolicy::NoRetention {
        return Err(RetrievalError::Internal(
            "visual provider must be local and no-retention".to_string(),
        ));
    }
    Ok(())
}

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

#[async_trait]
impl CandidateRetriever for VisualPageRegionRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        if request.plan.intent != maestria_domain::SearchIntent::VisualDocument {
            return Ok(CandidateBatch {
                descriptor: self.descriptor.clone(),
                query: request.query.q,
                candidates: Vec::new(),
                status: SearchLaneStatus::Empty,
                generation: Some(self.descriptor.generation),
                bytes_read: 0,
            });
        }
        if !scan_secrets(&request.query.q).is_clean() {
            return Err(RetrievalError::Internal(
                "visual query rejected by secret scanner".to_string(),
            ));
        }
        let identity: EmbeddingIdentity = self
            .embedding_provider
            .identity()
            .ok_or_else(|| RetrievalError::Internal("visual identity unavailable".to_string()))?;
        if identity != self.expected_identity {
            return Err(RetrievalError::Internal(
                "visual provider identity does not match active retriever capability".to_string(),
            ));
        }
        ensure_local_no_retention(self.embedding_provider.as_ref())?;
        let response = self
            .embedding_provider
            .embed_query(&request.query.q, identity.clone())
            .map_err(port_error)?;
        if response.identity != identity {
            return Err(RetrievalError::Internal(
                "visual provider response identity changed during query".to_string(),
            ));
        }
        if response.disclosure.remote
            || response.disclosure.retention != RetentionPolicy::NoRetention
        {
            return Err(RetrievalError::Internal(
                "visual provider violates local no-retention policy".to_string(),
            ));
        }
        self.retrieve_with_vector(
            VectorSearchQuery {
                vector: response.vector,
                limit: u32::try_from(request.query.limit).map_or(u32::MAX, |value| value),
                provider_id: Some(response.provider_id),
                model: Some(response.model),
                model_version: Some(response.model_version),
                identity: Some(response.identity),
            },
            request,
        )
    }
}

#[cfg(test)]
#[path = "visual_tests.rs"]
mod tests;
