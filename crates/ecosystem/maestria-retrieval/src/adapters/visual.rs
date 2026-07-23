use std::sync::Arc;

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, one_based_rank, port_error,
};
use super::score_provenance::dense_score;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};
use async_trait::async_trait;
use maestria_domain::{
    CorpusSnapshotId, EvidenceCandidate, EvidenceKind, IndexGenerationId, IndexGenerationRegistry,
    IndexStatus, RepresentationName, RetrievalReason, SearchLaneStatus, SourceSpan,
};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EvidenceRepository,
    RetentionPolicy, VectorIndex, VectorSearchQuery, VisualEmbeddingProvider,
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

    /// Returns the exact provider identity validated by this capability.
    pub fn identity(&self) -> &EmbeddingIdentity {
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
        raw_rank: u32,
        identity: &EmbeddingIdentity,
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
            dense_score(
                &self.descriptor,
                score,
                raw_rank,
                identity,
                "visual_cosine_similarity_micros",
            )?,
            vec![RetrievalReason::SemanticSimilarity],
        )
        .map(Some)
    }

    fn retrieve_with_vector(
        &self,
        vector: VectorSearchQuery,
        request: CandidateRequest,
        identity: &EmbeddingIdentity,
    ) -> Result<CandidateBatch, RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        let hits = self.index.search_similar(vector).map_err(port_error)?;
        let mut candidates = Vec::with_capacity(request.query.limit.min(hits.len()));
        for (index, hit) in hits.into_iter().enumerate() {
            let raw_rank = one_based_rank(index);
            let Some(candidate) = self.candidate_from_hit(hit, raw_rank, identity)? else {
                continue;
            };
            candidates.push(candidate);
            if candidates.len() >= request.query.limit {
                break;
            }
        }
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

pub(super) fn ensure_local_no_retention(
    provider: &dyn VisualEmbeddingProvider,
) -> Result<(), RetrievalError> {
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
                limit: match u32::try_from(request.query.limit) {
                    Ok(value) => value,
                    Err(e) => {
                        let _ = e;
                        u32::MAX
                    }
                },
                provider_id: Some(response.provider_id),
                model: Some(response.model),
                model_version: Some(response.model_version),
                identity: Some(response.identity),
            },
            request,
            &identity,
        )
    }
}

#[cfg(test)]
#[path = "visual_tests.rs"]
mod tests;
