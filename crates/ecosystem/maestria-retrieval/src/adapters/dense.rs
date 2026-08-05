use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{EvidenceCandidate, IndexGenerationId, SearchLaneStatus};
use maestria_governance::scan_secrets;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingInputKind, EmbeddingProvider,
    EmbeddingRequest, EvidenceRepository, VectorIndex, VectorSearchQuery,
};

use super::SourceSnapshotVerifier;
use super::chunk_access::{load_authorized_chunk, source_filter_allows_chunk};
use super::common::{candidate_from_records, generation_mismatch, one_based_rank, port_error};
use super::prescore_cache::PrescoreCache;
use super::score_provenance::dense_score;
use crate::traits::CandidateRetriever;
use crate::types::{
    CandidateBatch, CandidateRequest, CandidateSourceFilter, RetrievalError, RetrieverDescriptor,
};
#[cfg(test)]
#[path = "dense_tests.rs"]
mod tests;
type AuthorizedDenseRecord = (
    maestria_domain::Artifact,
    maestria_domain::Chunk,
    maestria_domain::Evidence,
);

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
    descriptor: RetrieverDescriptor,
}

impl DenseChunkRetriever {
    pub fn new(parts: DenseChunkRetrieverParts, generation: IndexGenerationId) -> Self {
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            embedding_provider: parts.embedding_provider,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
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
        let mut vector = vector;
        vector.execution_budget = request.execution_budget;
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        let identity = vector.identity.clone().ok_or_else(|| {
            RetrievalError::Internal("dense vector query identity unavailable".to_string())
        })?;
        let authorized = PrescoreCache::new(request.query.limit);
        let bounded = self
            .index
            .search_similar_filtered(vector, &|chunk_id| {
                self.prefilter_hit(
                    chunk_id,
                    &request.authorization,
                    &authorized,
                    request.source_filter.as_ref(),
                )
            })
            .map_err(port_error)?;
        let hits = bounded.hits;
        let mut candidates = Vec::with_capacity(hits.len());
        for (raw_rank, hit) in hits.into_iter().enumerate() {
            let Some(candidate) = self.candidate_from_hit(
                hit,
                one_based_rank(raw_rank)?,
                &identity,
                &request.authorization,
                request.source_filter.as_ref(),
                &authorized,
            )?
            else {
                continue;
            };
            candidates.push(candidate);
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
            execution: bounded.execution,
        })
    }

    fn prefilter_hit(
        &self,
        chunk_id: maestria_domain::ChunkId,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
        authorized: &PrescoreCache<AuthorizedDenseRecord>,
        source_filter: Option<&CandidateSourceFilter>,
    ) -> Result<bool, maestria_ports::PortError> {
        if !source_filter_allows_chunk(self.chunks.as_ref(), chunk_id, source_filter)? {
            return Ok(false);
        }
        let Some(record) = self.authorized_record(chunk_id, authorization)? else {
            return Ok(false);
        };
        authorized.insert(chunk_id, record);
        Ok(true)
    }

    fn authorized_record(
        &self,
        chunk_id: maestria_domain::ChunkId,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<Option<AuthorizedDenseRecord>, maestria_ports::PortError> {
        let Some((artifact, chunk)) = load_authorized_chunk(
            self.chunks.as_ref(),
            self.artifacts.as_ref(),
            chunk_id,
            authorization,
        )?
        else {
            return Ok(None);
        };
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id)? else {
            return Ok(None);
        };
        if evidence.artifact_id != artifact.id {
            return Err(maestria_ports::PortError::Conflict {
                message: format!(
                    "evidence {} owner mismatch: expected artifact {}, found {}",
                    evidence.id, artifact.id, evidence.artifact_id
                ),
            });
        }
        if authorization.evaluate(&evidence.security)
            != maestria_governance::RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        Ok(Some((artifact, chunk, evidence)))
    }
    fn candidate_from_hit(
        &self,
        hit: maestria_ports::VectorSearchHit,
        raw_rank: u32,
        identity: &maestria_ports::EmbeddingIdentity,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
        authorized: &PrescoreCache<AuthorizedDenseRecord>,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        if !source_filter_allows_chunk(self.chunks.as_ref(), hit.chunk_id, source_filter)
            .map_err(port_error)?
        {
            return Ok(None);
        }
        let record = match authorized.take(hit.chunk_id) {
            Some(record) => Some(record),
            None => self
                .authorized_record(hit.chunk_id, authorization)
                .map_err(port_error)?,
        };
        let Some((artifact, chunk, evidence)) = record else {
            return Ok(None);
        };
        self.verifier.verify(&evidence, &artifact)?;
        let score = if hit.score.is_finite() && hit.score > 0.0 {
            (hit.score.min(1.0) * 1_000_000.0).floor() as u32
        } else {
            0
        };
        candidate_from_records(
            artifact.id,
            artifact.content_hash.as_ref(),
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
        let disclosure = self.embedding_provider.disclosure();
        if disclosure.remote || disclosure.retention != maestria_ports::RetentionPolicy::NoRetention
        {
            return Err(RetrievalError::Internal(
                "dense embedding provider must be local and no-retention".to_string(),
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
                model: identity.fingerprint.model.as_str().to_string(),
                kind: EmbeddingInputKind::Query,
                identity,
            })
            .map_err(port_error)?;
        let limit = u32::try_from(request.query.limit).map_err(|_| {
            RetrievalError::Internal(
                "dense result limit exceeds vector query representation".into(),
            )
        })?;
        let execution_budget = request.execution_budget;
        self.retrieve_with_vector(
            request,
            VectorSearchQuery {
                vector: response.vector,
                limit,
                provider_id: Some(response.provider_id),
                model: Some(response.model),
                model_version: Some(response.model_version),
                execution_budget,
                identity: Some(response.identity),
            },
        )
    }
}
