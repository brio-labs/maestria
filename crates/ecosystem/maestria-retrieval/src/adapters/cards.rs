use std::cell::Cell;
use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{EvidenceCandidate, IndexGenerationId, IndexStatus, SearchLaneStatus};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EvidenceRepository,
    FullTextIndex, SearchQuery,
};

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, one_based_rank, port_error,
};
use super::score_provenance::lexical_score;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};

/// Port-backed card retrieval that emits source-grounded evidence candidates.
pub struct CardRetriever {
    index: Arc<dyn FullTextIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    cards: Arc<dyn CardRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    verifier: SourceSnapshotVerifier,
    policy: RetrievalSecurityPolicy,
    descriptor: RetrieverDescriptor,
}

pub struct CardRetrieverParts {
    pub index: Arc<dyn FullTextIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub cards: Arc<dyn CardRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
}

impl CardRetriever {
    pub fn new(
        parts: CardRetrieverParts,
        policy: RetrievalSecurityPolicy,
        generation: IndexGenerationId,
    ) -> Self {
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            cards: parts.cards,
            chunks: parts.chunks,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            policy,
            descriptor: RetrieverDescriptor {
                id: "cards".to_string(),
                modality: "text".to_string(),
                representation: maestria_domain::RepresentationName::new("lexical_text_v1"),
                generation,
            },
        }
    }

    pub fn search(
        &self,
        query: SearchQuery,
    ) -> Result<Vec<maestria_ports::CardHit>, RetrievalError> {
        let hits = self.filtered_hits(query)?;
        let mut allowed = Vec::with_capacity(hits.len());
        for (raw_rank, hit) in hits.into_iter().enumerate() {
            if self
                .candidate_from_hit(&hit, one_based_rank(raw_rank))?
                .is_some()
            {
                allowed.push(hit);
            }
        }
        Ok(allowed)
    }

    fn filtered_hits(
        &self,
        query: SearchQuery,
    ) -> Result<Vec<maestria_ports::CardHit>, RetrievalError> {
        let filter_error = Cell::new(None);
        let hits = self
            .index
            .search_cards_filtered(query, &|card_id, artifact_id| match self
                .prefilter_hit(card_id, artifact_id)
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
        Ok(hits)
    }

    fn prefilter_hit(
        &self,
        card_id: maestria_domain::CardId,
        artifact_id: maestria_domain::ArtifactId,
    ) -> Result<bool, maestria_ports::PortError> {
        let Some(artifact) = self.artifacts.get(artifact_id)? else {
            return Ok(false);
        };
        let Some(card) = self.cards.get(card_id)? else {
            return Ok(false);
        };
        if card.artifact_id != artifact.id
            || artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || self.policy.evaluate(&card.security) != RetrievalDecision::Allowed
            || !scan_secrets(&card.body).is_clean()
        {
            return Ok(false);
        }
        let mut chunks = self.chunks.list_for_artifact(card.artifact_id)?;
        chunks.sort_by_key(|chunk| (chunk.order, chunk.id));
        let Some(chunk) = chunks
            .into_iter()
            .find(|chunk| chunk.node_id == card.node_id)
        else {
            return Ok(false);
        };
        if !scan_secrets(&chunk.text).is_clean() {
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
        hit: &maestria_ports::CardHit,
        raw_rank: u32,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let Some(artifact) = self
            .artifacts
            .get(hit.card.artifact_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        let Some(card) = self.cards.get(hit.card.card_id).map_err(port_error)? else {
            return Ok(None);
        };
        if artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || self.policy.evaluate(&card.security) != RetrievalDecision::Allowed
            || !scan_secrets(&card.body).is_clean()
        {
            return Ok(None);
        }
        let mut chunks = self
            .chunks
            .list_for_artifact(card.artifact_id)
            .map_err(port_error)?;
        chunks.sort_by_key(|chunk| (chunk.order, chunk.id));
        let Some(chunk) = chunks
            .into_iter()
            .find(|chunk| chunk.node_id == card.node_id)
        else {
            return Ok(None);
        };
        if !scan_secrets(&chunk.text).is_clean() {
            return Ok(None);
        }
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
            return Ok(None);
        };
        if self.policy.evaluate(&evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        self.verifier.verify(&evidence)?;
        candidate_from_records(
            artifact.id,
            &chunk.source_span,
            &evidence,
            card.node_id,
            lexical_score(&self.descriptor, hit.score, raw_rank)?,
            vec![maestria_domain::RetrievalReason::LexicalMatch],
        )
        .map(Some)
    }
}

#[async_trait]
impl CandidateRetriever for CardRetriever {
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
        let hits = self.filtered_hits(request.query.clone())?;
        let mut bytes_read = 0_u64;
        let mut candidates = Vec::with_capacity(hits.len());
        for (raw_rank, hit) in hits.into_iter().enumerate() {
            let Some(candidate) = self.candidate_from_hit(&hit, one_based_rank(raw_rank))? else {
                continue;
            };
            bytes_read = bytes_read.saturating_add(
                (candidate.source_span.range().end - candidate.source_span.range().start) as u64,
            );
            candidates.push(candidate);
            if candidates.len() >= request.query.limit {
                break;
            }
        }
        Ok(CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            status: if candidates.is_empty() {
                SearchLaneStatus::Empty
            } else {
                SearchLaneStatus::Succeeded
            },
            generation: Some(self.descriptor.generation),
            candidates,
            bytes_read,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filtered_test_support::{FilteredFullTextSpy, denied_artifact, request};
    use crate::traits::CandidateRetriever;
    use maestria_domain::{ArtifactId, CardId, ChunkId, SearchIntent};
    use maestria_ports::{
        InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
        InMemoryChunkRepository, InMemoryEvidenceRepository,
    };

    #[tokio::test]
    async fn denied_card_candidates_are_filtered_before_scoring()
    -> Result<(), Box<dyn std::error::Error>> {
        let generation = IndexGenerationId::new(1);
        let artifact_id = ArtifactId::new(7);
        let index = Arc::new(FilteredFullTextSpy::new(
            ChunkId::new(11),
            CardId::new(12),
            artifact_id,
        ));
        let artifacts = InMemoryArtifactRepository::new();
        artifacts.put(denied_artifact(artifact_id))?;
        let retriever = CardRetriever::new(
            CardRetrieverParts {
                index: index.clone(),
                artifacts: Arc::new(artifacts),
                cards: Arc::new(InMemoryCardRepository::new()),
                chunks: Arc::new(InMemoryChunkRepository::new()),
                evidence: Arc::new(InMemoryEvidenceRepository::new()),
                blobs: Arc::new(InMemoryBlobStore::new()),
            },
            RetrievalSecurityPolicy::default(),
            generation,
        );

        let batch = retriever
            .retrieve(request(SearchIntent::FactualLocal, generation)?)
            .await?;
        assert_eq!(index.card_filter_calls(), 1);
        assert_eq!(index.card_score_calls(), 0);
        assert!(batch.candidates.is_empty());
        Ok(())
    }
}
