use std::cell::Cell;
use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{EvidenceCandidate, IndexGenerationId, IndexStatus, SearchLaneStatus};
use maestria_governance::{RetrievalDecision, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, FullTextIndex,
};

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, one_based_rank, port_error,
};
use super::score_provenance::lexical_score;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};

/// Dependencies required by the lexical chunk adapter.
pub struct LexicalChunkRetrieverParts {
    pub index: Arc<dyn FullTextIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
}

/// Port-backed lexical chunk retrieval with policy and provenance checks.
pub struct LexicalChunkRetriever {
    index: Arc<dyn FullTextIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    verifier: SourceSnapshotVerifier,
    descriptor: RetrieverDescriptor,
}

impl LexicalChunkRetriever {
    pub fn new(parts: LexicalChunkRetrieverParts, generation: IndexGenerationId) -> Self {
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            descriptor: RetrieverDescriptor {
                id: "lexical_chunks".to_string(),
                modality: "text".to_string(),
                representation: maestria_domain::RepresentationName::new("lexical_text_v1"),
                generation,
            },
        }
    }
}

#[async_trait]
impl CandidateRetriever for LexicalChunkRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        let filter_error = Cell::new(None);
        let hits = self
            .index
            .search_filtered(request.query.clone(), &|chunk_id, artifact_id| match self
                .prefilter_hit(chunk_id, artifact_id, &request.authorization)
            {
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
            let raw_rank = one_based_rank(raw_rank);
            let Some(candidate) = self.candidate_from_hit(hit, raw_rank, &request.authorization)?
            else {
                continue;
            };
            let span_len = candidate
                .source_span
                .range()
                .end
                .saturating_sub(candidate.source_span.range().start);
            bytes_read = bytes_read.saturating_add(span_len as u64);
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
}
impl LexicalChunkRetriever {
    fn prefilter_hit(
        &self,
        chunk_id: maestria_domain::ChunkId,
        artifact_id: maestria_domain::ArtifactId,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<bool, maestria_ports::PortError> {
        let Some(artifact) = self.artifacts.get(artifact_id)? else {
            return Ok(false);
        };
        if artifact.index_status != IndexStatus::Indexed
            || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
        {
            return Ok(false);
        }
        let Some(chunk) = self.chunks.get(chunk_id)? else {
            return Ok(false);
        };
        if chunk.artifact_id != artifact.id || !scan_secrets(&chunk.text).is_clean() {
            return Ok(false);
        }
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id)? else {
            return Ok(false);
        };
        Ok(
            authorization.evaluate(&evidence.security) == RetrievalDecision::Allowed
                && scan_secrets(&evidence.excerpt).is_clean(),
        )
    }
}

impl LexicalChunkRetriever {
    fn candidate_from_hit(
        &self,
        hit: maestria_ports::SearchHit,
        raw_rank: u32,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let Some(artifact) = self
            .artifacts
            .get(hit.chunk.artifact_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        if artifact.index_status != IndexStatus::Indexed
            || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
        {
            return Ok(None);
        }
        if !scan_secrets(&hit.chunk.text).is_clean() {
            return Ok(None);
        }
        let Some(chunk) = self.chunks.get(hit.chunk.chunk_id).map_err(port_error)? else {
            return Ok(None);
        };
        if chunk.artifact_id != artifact.id {
            return Ok(None);
        }
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
            return Ok(None);
        };
        if authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        self.verifier.verify(&evidence)?;
        candidate_from_records(
            artifact.id,
            &chunk.source_span,
            &evidence,
            chunk.node_id,
            lexical_score(&self.descriptor, hit.score, raw_rank)?,
            vec![maestria_domain::RetrievalReason::LexicalMatch],
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filtered_test_support::{FilteredFullTextSpy, denied_artifact, request};
    use crate::traits::CandidateRetriever;
    use maestria_domain::{ArtifactId, ChunkId, SearchIntent};
    use maestria_ports::{
        InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
        InMemoryEvidenceRepository,
    };

    #[tokio::test]
    async fn denied_lexical_candidates_are_filtered_before_scoring()
    -> Result<(), Box<dyn std::error::Error>> {
        let generation = IndexGenerationId::new(1);
        let artifact_id = ArtifactId::new(7);
        let index = Arc::new(FilteredFullTextSpy::new(
            ChunkId::new(11),
            maestria_domain::CardId::new(12),
            artifact_id,
        ));
        let artifacts = InMemoryArtifactRepository::new();
        artifacts.put(denied_artifact(artifact_id))?;
        let retriever = LexicalChunkRetriever::new(
            LexicalChunkRetrieverParts {
                index: index.clone(),
                artifacts: Arc::new(artifacts),
                chunks: Arc::new(InMemoryChunkRepository::new()),
                evidence: Arc::new(InMemoryEvidenceRepository::new()),
                blobs: Arc::new(InMemoryBlobStore::new()),
            },
            generation,
        );

        let batch = retriever
            .retrieve(request(SearchIntent::FactualLocal, generation)?)
            .await?;
        assert_eq!(index.chunk_filter_calls(), 1);
        assert_eq!(index.chunk_score_calls(), 0);
        assert!(batch.candidates.is_empty());
        Ok(())
    }
}
