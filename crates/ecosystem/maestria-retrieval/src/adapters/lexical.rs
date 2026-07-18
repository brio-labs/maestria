use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{EvidenceCandidate, IndexGenerationId, IndexStatus, SearchLaneStatus};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, FullTextIndex,
};

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, port_error,
};
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
    policy: RetrievalSecurityPolicy,
    descriptor: RetrieverDescriptor,
}

impl LexicalChunkRetriever {
    pub fn new(
        parts: LexicalChunkRetrieverParts,
        policy: RetrievalSecurityPolicy,
        generation: IndexGenerationId,
    ) -> Self {
        Self {
            index: parts.index,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            policy,
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
        let hits = self
            .index
            .search(request.query.clone())
            .map_err(port_error)?;
        let mut candidates = Vec::with_capacity(hits.len());
        let mut bytes_read = 0_u64;
        for hit in hits {
            let Some(candidate) = self.candidate_from_hit(hit)? else {
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
    fn candidate_from_hit(
        &self,
        hit: maestria_ports::SearchHit,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let Some(artifact) = self
            .artifacts
            .get(hit.chunk.artifact_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        if artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
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
            chunk.node_id,
            hit.score,
        )
        .map(Some)
    }
}
