use std::sync::Arc;

use sillage_domain::{
    Card, Chunk, EvidenceCandidate, IndexGenerationId, IndexStatus, SearchLaneStatus,
};
use sillage_governance::{RetrievalDecision, scan_secrets};
use sillage_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EvidenceRepository,
    FullTextIndex, SearchQuery,
};

use super::SourceSnapshotVerifier;
use super::common::{candidate_from_records, generation_mismatch, one_based_rank, port_error};
use super::score_provenance::lexical_score;
use crate::traits::CandidateRetriever;
use crate::types::{
    CandidateBatch, CandidateRequest, CandidateSourceFilter, RetrievalError, RetrieverDescriptor,
};

/// Port-backed card retrieval that emits source-grounded evidence candidates.
pub struct CardRetriever {
    index: Arc<dyn FullTextIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    cards: Arc<dyn CardRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    verifier: SourceSnapshotVerifier,
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
    pub fn new(parts: CardRetrieverParts, generation: IndexGenerationId) -> Self {
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            cards: parts.cards,
            chunks: parts.chunks,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            descriptor: RetrieverDescriptor {
                id: "cards".to_string(),
                modality: "text".to_string(),
                representation: sillage_domain::RepresentationName::new("lexical_text_v1"),
                generation,
            },
        }
    }
    fn filtered_hits(
        &self,
        query: SearchQuery,
        authorization: &sillage_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
    ) -> Result<sillage_ports::BoundedSearch<sillage_ports::CardHit>, RetrievalError> {
        let hits = self
            .index
            .search_cards_filtered(query, &|card_id, artifact_id| {
                self.prefilter_hit(card_id, artifact_id, authorization, source_filter)
            })
            .map_err(port_error)?;
        Ok(hits)
    }

    /// Structural cards can refer to a tree root rather than a chunk node.
    /// Such cards must identify one exact same-artifact source span.
    fn source_chunk(&self, card: &Card) -> Result<Option<Chunk>, sillage_ports::PortError> {
        let chunks = self.chunks.list_for_artifact(card.artifact_id)?;
        let node_match = chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| {
                chunk.artifact_id == card.artifact_id && chunk.node_id == card.node_id
            })
            .min_by_key(|(_, chunk)| (chunk.order, chunk.id))
            .map(|(index, _)| index);
        if let Some(index) = node_match {
            return Ok(chunks.into_iter().nth(index));
        }
        let mut spans = chunks.into_iter().filter(|chunk| {
            chunk.artifact_id == card.artifact_id && chunk.source_span == card.source_span
        });
        let Some(chunk) = spans.next() else {
            return Ok(None);
        };
        Ok(spans.next().is_none().then_some(chunk))
    }

    fn prefilter_hit(
        &self,
        card_id: sillage_domain::CardId,
        artifact_id: sillage_domain::ArtifactId,
        authorization: &sillage_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
    ) -> Result<bool, sillage_ports::PortError> {
        if source_filter.is_some_and(|filter| !filter.allows(artifact_id)) {
            return Ok(false);
        }
        let Some(artifact) = self.artifacts.get(artifact_id)? else {
            return Ok(false);
        };
        let Some(card) = self.cards.get(card_id)? else {
            return Ok(false);
        };
        if card.artifact_id != artifact.id
            || artifact.index_status != IndexStatus::Indexed
            || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || authorization.evaluate(&card.security) != RetrievalDecision::Allowed
            || !scan_secrets(&card.body).is_clean()
        {
            return Ok(false);
        }
        let Some(chunk) = self.source_chunk(&card)? else {
            return Ok(false);
        };
        if !scan_secrets(&chunk.text).is_clean() {
            return Ok(false);
        }
        let evidence_id = sillage_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id)? else {
            return Ok(false);
        };
        Ok(
            authorization.evaluate(&evidence.security) == RetrievalDecision::Allowed
                && scan_secrets(&evidence.excerpt).is_clean(),
        )
    }
    fn candidate_from_hit(
        &self,
        hit: &sillage_ports::CardHit,
        raw_rank: u32,
        authorization: &sillage_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        if source_filter.is_some_and(|filter| !filter.allows(hit.card.artifact_id)) {
            return Ok(None);
        }
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
            || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || authorization.evaluate(&card.security) != RetrievalDecision::Allowed
            || !scan_secrets(&card.body).is_clean()
        {
            return Ok(None);
        }
        let Some(chunk) = self.source_chunk(&card).map_err(port_error)? else {
            return Ok(None);
        };
        if !scan_secrets(&chunk.text).is_clean() {
            return Ok(None);
        }
        let evidence_id = sillage_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
            return Ok(None);
        };
        if authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        self.verifier.verify(&evidence, &artifact)?;
        candidate_from_records(
            artifact.id,
            artifact.content_hash.as_ref(),
            &chunk.source_span,
            &evidence,
            chunk.node_id,
            lexical_score(&self.descriptor, hit.score, raw_rank)?,
            vec![sillage_domain::RetrievalReason::LexicalMatch],
        )
        .map(Some)
    }
}
impl CandidateRetriever for CardRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        let authorization = request.authorization.clone();
        let mut query = request.query.clone();
        query.execution_budget = request.execution_budget;
        let bounded = self.filtered_hits(query, &authorization, request.source_filter.as_ref())?;
        let mut candidates = Vec::with_capacity(bounded.hits.len());
        for (raw_rank, hit) in bounded.hits.into_iter().enumerate() {
            let Some(candidate) = self.candidate_from_hit(
                &hit,
                one_based_rank(raw_rank)?,
                &authorization,
                request.source_filter.as_ref(),
            )?
            else {
                continue;
            };
            candidates.push(candidate);
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
            execution: bounded.execution,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::filtered_test_support::{FilteredFullTextSpy, denied_artifact, request};
    use crate::traits::CandidateRetriever;
    use sillage_domain::{ArtifactId, CardId, ChunkId, SearchIntent};
    use sillage_ports::{
        InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
        InMemoryChunkRepository, InMemoryEvidenceRepository,
    };

    #[test]
    fn denied_card_candidates_are_filtered_before_scoring() -> Result<(), Box<dyn std::error::Error>>
    {
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
            generation,
        );

        let batch = retriever.retrieve(request(SearchIntent::FactualLocal, generation)?)?;
        assert_eq!(index.card_filter_calls(), 1);
        assert_eq!(index.card_score_calls(), 0);
        assert!(batch.candidates.is_empty());
        Ok(())
    }
}

#[cfg(test)]
#[path = "card_source_span_tests.rs"]
mod source_span_tests;
