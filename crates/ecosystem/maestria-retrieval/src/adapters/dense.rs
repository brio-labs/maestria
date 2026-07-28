use std::cell::Cell;
use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{EvidenceCandidate, IndexGenerationId, IndexStatus, SearchLaneStatus};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingInputKind, EmbeddingProvider,
    EmbeddingRequest, EvidenceRepository, VectorIndex, VectorSearchQuery,
};

use super::common::{
    SourceSnapshotVerifier, bounded_candidate_bytes, candidate_from_records, generation_mismatch,
    one_based_rank, port_error,
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
        let filter_error = Cell::new(None);
        let hits = self
            .index
            .search_similar_filtered(vector, &|chunk_id| match self.prefilter_hit(chunk_id) {
                Ok(allowed) => allowed,
                Err(error) => {
                    filter_error.set(Some(error));
                    false
                }
            })
            .map_err(port_error)?;
        if let Some(error) = filter_error.take() {
            return Err(port_error(error));
        }
        let mut candidates = Vec::with_capacity(hits.len());
        let mut bytes_read = 0_u64;
        for (raw_rank, hit) in hits.into_iter().enumerate() {
            let Some(candidate) =
                self.candidate_from_hit(hit, one_based_rank(raw_rank), &identity)?
            else {
                continue;
            };
            bytes_read = bytes_read.saturating_add(bounded_candidate_bytes(&candidate));
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
            bytes_read,
        })
    }

    fn prefilter_hit(
        &self,
        chunk_id: maestria_domain::ChunkId,
    ) -> Result<bool, maestria_ports::PortError> {
        let Some(chunk) = self.chunks.get(chunk_id)? else {
            return Ok(false);
        };
        let Some(artifact) = self.artifacts.get(chunk.artifact_id)? else {
            return Ok(false);
        };
        if artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || !scan_secrets(&chunk.text).is_clean()
        {
            return Ok(false);
        }
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id)? else {
            return Ok(false);
        };
        Ok(
            self.policy.evaluate(&evidence.security) == RetrievalDecision::Allowed
                && scan_secrets(&evidence.excerpt).is_clean(),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filtered_test_support::{
        FilteredVectorSpy, chunk, denied_artifact, request,
    };
    use maestria_ports::{
        EmbeddingProvenance, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse,
        InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
        InMemoryEvidenceRepository, InMemoryVectorIndex, VectorEmbedding, VectorIndex,
    };

    struct UnusedEmbeddingProvider;

    impl EmbeddingProvider for UnusedEmbeddingProvider {
        fn embed(
            &self,
            _request: EmbeddingRequest,
        ) -> Result<EmbeddingResponse, maestria_ports::PortError> {
            Err(maestria_ports::PortError::Downstream {
                message: "embedding provider must not be called".to_string(),
            })
        }
    }

    #[test]
    fn denied_dense_candidates_are_filtered_before_scoring()
    -> Result<(), Box<dyn std::error::Error>> {
        let generation = IndexGenerationId::new(1);
        let artifact_id = maestria_domain::ArtifactId::new(7);
        let chunk_id = maestria_domain::ChunkId::new(11);
        let index = Arc::new(FilteredVectorSpy::new(chunk_id));
        let artifacts = InMemoryArtifactRepository::new();
        artifacts.put(denied_artifact(artifact_id))?;
        let chunks = InMemoryChunkRepository::new();
        chunks.put(chunk(
            chunk_id,
            artifact_id,
            maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
        ))?;
        let retriever = DenseChunkRetriever::new(
            DenseChunkRetrieverParts {
                index: index.clone(),
                artifacts: Arc::new(artifacts),
                chunks: Arc::new(chunks),
                evidence: Arc::new(InMemoryEvidenceRepository::new()),
                blobs: Arc::new(InMemoryBlobStore::new()),
                embedding_provider: Arc::new(UnusedEmbeddingProvider),
            },
            RetrievalSecurityPolicy::default(),
            generation,
        );
        let identity = maestria_ports::EmbeddingIdentity::legacy("dense-test", 1)?;
        let batch = retriever.retrieve_with_vector(
            request(maestria_domain::SearchIntent::FactualLocal, generation)?,
            VectorSearchQuery {
                vector: vec![1.0],
                limit: 5,
                identity: Some(identity.clone()),
                provider_id: None,
                model: None,
                model_version: None,
            },
        )?;
        assert_eq!(index.filter_calls(), 1);
        assert_eq!(index.score_calls(), 0);
        assert!(batch.candidates.is_empty());
        Ok(())
    }

    #[test]
    fn dense_batch_reports_bounded_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let generation = IndexGenerationId::new(1);
        let artifact_id = maestria_domain::ArtifactId::new(7);
        let chunk_id = maestria_domain::ChunkId::new(11);
        let source = b"alpha\nbeta\n";
        let blobs = InMemoryBlobStore::new();
        let snapshot = blobs.put(source.to_vec())?;
        let content_hash = maestria_domain::content_hash(source);
        let artifacts = InMemoryArtifactRepository::new();
        artifacts.put(maestria_domain::Artifact {
            id: artifact_id,
            title: "dense".to_string(),
            chunk_ids: std::iter::once(chunk_id).collect(),
            card_ids: Default::default(),
            claim_ids: Default::default(),
            evidence_ids: Default::default(),
            index_status: IndexStatus::Indexed,
            content_hash: Some(content_hash.clone()),
            parse_status: None,
            security: Default::default(),
        })?;
        let chunks = InMemoryChunkRepository::new();
        chunks.put(maestria_domain::Chunk {
            id: chunk_id,
            artifact_id,
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 2,
            },
            representations: Vec::new(),
            order: 0,
            text: "alpha".to_string(),
        })?;
        let evidence = InMemoryEvidenceRepository::new();
        evidence.put(maestria_domain::Evidence {
            id: maestria_domain::evidence_id_for(artifact_id, 0),
            artifact_id,
            claim_id: None,
            kind: maestria_domain::EvidenceKind::FileSpan {
                path: "dense.md".to_string(),
                range: maestria_domain::ContentRange { start: 1, end: 2 },
                content_hash,
                snapshot: Some(snapshot),
            },
            excerpt: "alpha".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: Default::default(),
        })?;
        let identity = maestria_ports::EmbeddingIdentity::legacy("dense-test", 1)?;
        let index = InMemoryVectorIndex::new();
        index.index_embeddings(vec![VectorEmbedding {
            chunk_id,
            vector: vec![1.0],
            provenance: EmbeddingProvenance {
                content_hash: "embedding".to_string(),
                identity: identity.clone(),
                provider_id: "dense-test".to_string(),
                model: "dense-test".to_string(),
                model_version: "1".to_string(),
                disclosure: maestria_ports::ProviderDisclosure {
                    remote: false,
                    retention: maestria_ports::RetentionPolicy::NoRetention,
                },
            },
        }])?;
        let retriever = DenseChunkRetriever::new(
            DenseChunkRetrieverParts {
                index: Arc::new(index),
                artifacts: Arc::new(artifacts),
                chunks: Arc::new(chunks),
                evidence: Arc::new(evidence),
                blobs: Arc::new(blobs),
                embedding_provider: Arc::new(UnusedEmbeddingProvider),
            },
            RetrievalSecurityPolicy::default(),
            generation,
        );
        let batch = retriever.retrieve_with_vector(
            request(maestria_domain::SearchIntent::FactualLocal, generation)?,
            VectorSearchQuery {
                vector: vec![1.0],
                limit: 5,
                provider_id: Some("dense-test".to_string()),
                model: Some("dense-test".to_string()),
                model_version: Some("1".to_string()),
                identity: Some(identity),
            },
        )?;
        assert_eq!(batch.candidates.len(), 1);
        assert_eq!(batch.bytes_read, 1);
        Ok(())
    }
}
