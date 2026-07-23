use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{EvidenceCandidate, IndexGenerationId, IndexStatus, SearchLaneStatus};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingInputKind, EmbeddingProvider,
    EmbeddingRequest, EvidenceRepository, VectorIndex, VectorSearchQuery,
};

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, one_based_rank, port_error,
};
use super::score_provenance::dense_score;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};

/// Dependencies required by the dense chunk adapter.
pub struct DenseChunkRetrieverParts {
    pub index: Arc<dyn VectorIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
    pub embedding_provider: Arc<dyn EmbeddingProvider + Send + Sync>,
}

/// Dense chunk retrieval keeps vector provenance separate from lexical lanes.
pub struct DenseChunkRetriever {
    index: Arc<dyn VectorIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    embedding_provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    verifier: SourceSnapshotVerifier,
    policy: RetrievalSecurityPolicy,
    descriptor: RetrieverDescriptor,
}

impl DenseChunkRetriever {
    pub fn new(
        parts: DenseChunkRetrieverParts,
        policy: RetrievalSecurityPolicy,
        generation: IndexGenerationId,
    ) -> Self {
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            embedding_provider: parts.embedding_provider,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            policy,
            descriptor: RetrieverDescriptor {
                id: "dense_chunks".to_string(),
                modality: "dense".to_string(),
                representation: maestria_domain::RepresentationName::new("dense_text_v1"),
                generation,
            },
        }
    }

    pub fn retrieve_with_vector(
        &self,
        request: CandidateRequest,
        vector: VectorSearchQuery,
    ) -> Result<CandidateBatch, RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        let identity = vector.identity.clone().ok_or_else(|| {
            RetrievalError::Internal("dense vector query identity unavailable".to_string())
        })?;
        let hits = self.index.search_similar(vector).map_err(port_error)?;
        let mut candidates = Vec::with_capacity(hits.len());
        for (raw_rank, hit) in hits.into_iter().enumerate() {
            let Some(candidate) =
                self.candidate_from_hit(hit, one_based_rank(raw_rank), &identity)?
            else {
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

    fn candidate_from_hit(
        &self,
        hit: maestria_ports::VectorSearchHit,
        raw_rank: u32,
        identity: &maestria_ports::EmbeddingIdentity,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let Some(chunk) = self.chunks.get(hit.chunk_id).map_err(port_error)? else {
            return Ok(None);
        };
        let Some(artifact) = self.artifacts.get(chunk.artifact_id).map_err(port_error)? else {
            return Ok(None);
        };
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
            return Ok(None);
        };
        if artifact.index_status != IndexStatus::Indexed
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
                "cosine_similarity_micros",
            )?,
            vec![maestria_domain::RetrievalReason::SemanticSimilarity],
        )
        .map(Some)
    }
}

#[async_trait]
impl CandidateRetriever for DenseChunkRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        if !scan_secrets(&request.query.q).is_clean() {
            return Err(RetrievalError::Internal(
                "dense query rejected by secret scanner".to_string(),
            ));
        }
        let identity = self
            .embedding_provider
            .identity()
            .ok_or_else(|| RetrievalError::Internal("dense identity unavailable".to_string()))?;
        let response = self
            .embedding_provider
            .embed(EmbeddingRequest {
                text: request.query.q.clone(),
                model: identity.fingerprint.model.clone(),
                kind: EmbeddingInputKind::Query,
                identity,
            })
            .map_err(port_error)?;
        let limit = match u32::try_from(request.query.limit) {
            Ok(value) => value,
            Err(e) => {
                let _ = e;
                u32::MAX
            }
        };
        self.retrieve_with_vector(
            request,
            VectorSearchQuery {
                vector: response.vector,
                limit,
                provider_id: Some(response.provider_id),
                model: Some(response.model),
                model_version: Some(response.model_version),
                identity: Some(response.identity),
            },
        )
    }
}
