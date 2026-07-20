use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, IndexStatus, LearnedSparseContribution, LearnedSparseReason,
    RetrievalModelFingerprint, RetrievalReason, SearchLaneStatus,
};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, LearnedSparseIndex,
    LearnedSparseProvider, RetentionPolicy, SparseIdentity, SparseInputKind, SparseSearchHit,
    SparseSearchQuery,
};

use super::common::{
    SourceSnapshotVerifier, candidate_from_records, generation_mismatch, port_error,
};
use super::learned_sparse_generation::LearnedSparseGenerationCapability;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};

pub struct LearnedSparseChunkRetrieverParts {
    pub index: Arc<dyn LearnedSparseIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
    pub provider: Arc<dyn LearnedSparseProvider + Send + Sync>,
}

pub struct LearnedSparseChunkRetriever {
    index: Arc<dyn LearnedSparseIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    provider: Arc<dyn LearnedSparseProvider + Send + Sync>,
    verifier: SourceSnapshotVerifier,
    policy: RetrievalSecurityPolicy,
    identity: SparseIdentity,
    fingerprint: RetrievalModelFingerprint,
    descriptor: RetrieverDescriptor,
}

impl LearnedSparseChunkRetriever {
    pub fn new(
        parts: LearnedSparseChunkRetrieverParts,
        policy: RetrievalSecurityPolicy,
        capability: LearnedSparseGenerationCapability,
    ) -> Result<Self, RetrievalError> {
        let identity = capability.identity().clone();
        let provider_identity = parts.provider.identity().ok_or_else(|| {
            RetrievalError::Internal("sparse provider identity unavailable".into())
        })?;
        if provider_identity != identity {
            return Err(RetrievalError::Internal(
                "sparse provider identity does not match retriever generation".into(),
            ));
        }
        let index_identity = parts
            .index
            .identity()
            .ok_or_else(|| RetrievalError::Internal("sparse index identity unavailable".into()))?;
        if index_identity != identity {
            return Err(RetrievalError::Internal(
                "sparse index identity does not match retriever generation".into(),
            ));
        }
        let fingerprint = RetrievalModelFingerprint::new(format!(
            "sparse:{}:{}:{}:{}:{}",
            identity.fingerprint.provider,
            identity.fingerprint.model,
            identity.fingerprint.revision,
            identity.fingerprint.vocabulary_hash.as_str(),
            identity.fingerprint.preprocessing_version
        ))
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        let descriptor = RetrieverDescriptor {
            id: "learned_sparse_chunks".to_string(),
            modality: "sparse".to_string(),
            representation: identity.representation.clone(),
            generation: identity.generation_id,
        };
        Ok(Self {
            index: parts.index,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            provider: parts.provider,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            policy,
            identity,
            fingerprint,
            descriptor,
        })
    }

    fn preflight(&self, request: &CandidateRequest) -> Result<(), RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        if request.plan.corpus_snapshot != self.identity.corpus_snapshot {
            return Err(RetrievalError::Internal(
                "sparse query corpus snapshot does not match its identity".into(),
            ));
        }
        if !scan_secrets(&request.query.q).is_clean() {
            return Err(RetrievalError::Internal(
                "sparse query rejected by secret scanner".into(),
            ));
        }
        let disclosure = self.provider.disclosure().ok_or_else(|| {
            RetrievalError::Internal("sparse provider disclosure unavailable".into())
        })?;
        if disclosure.remote || disclosure.retention != RetentionPolicy::NoRetention {
            return Err(RetrievalError::Internal(
                "sparse provider must be local and no-retention for this route".into(),
            ));
        }
        if self.provider.identity().as_ref() != Some(&self.identity) {
            return Err(RetrievalError::Internal(
                "sparse provider identity changed after construction".into(),
            ));
        }
        if self.index.identity().as_ref() != Some(&self.identity) {
            return Err(RetrievalError::Internal(
                "sparse index identity changed after construction".into(),
            ));
        }
        Ok(())
    }

    fn chunk_allowed(&self, chunk_id: maestria_domain::ChunkId) -> bool {
        self.checked_records(chunk_id)
            .is_ok_and(|records| records.is_some())
    }

    fn checked_records(
        &self,
        chunk_id: maestria_domain::ChunkId,
    ) -> Result<
        Option<(
            maestria_domain::Artifact,
            maestria_domain::Chunk,
            maestria_domain::Evidence,
        )>,
        RetrievalError,
    > {
        let Some(chunk) = self.chunks.get(chunk_id).map_err(port_error)? else {
            return Ok(None);
        };
        let Some(artifact) = self.artifacts.get(chunk.artifact_id).map_err(port_error)? else {
            return Ok(None);
        };
        if artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || !scan_secrets(&chunk.text).is_clean()
        {
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
        Ok(Some((artifact, chunk, evidence)))
    }

    fn candidate_from_hit(
        &self,
        hit: SparseSearchHit,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let Some((artifact, chunk, evidence)) = self.checked_records(hit.chunk_id)? else {
            return Ok(None);
        };
        let mut candidate =
            candidate_from_records(artifact.id, &chunk.source_span, &evidence, chunk.node_id, 0)?;
        candidate.reasons = vec![RetrievalReason::LearnedSparse(Box::new(
            LearnedSparseReason {
                score_micros: hit.score_micros,
                representation: self.identity.representation.clone(),
                fingerprint: self.fingerprint.clone(),
                contributions: hit
                    .contributions
                    .into_iter()
                    .map(|contribution| LearnedSparseContribution {
                        term_id: contribution.term_id,
                        contribution_micros: contribution.contribution_micros,
                    })
                    .collect(),
            },
        ))];
        Ok(Some(candidate))
    }
}

#[async_trait]
impl CandidateRetriever for LearnedSparseChunkRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        self.preflight(&request)?;
        let vector = self
            .provider
            .encode(
                &request.query.q,
                SparseInputKind::Query,
                self.identity.clone(),
            )
            .map_err(port_error)?;
        if vector.identity() != &self.identity {
            return Err(RetrievalError::Internal(
                "sparse provider returned an incompatible query identity".into(),
            ));
        }
        let limit = u32::try_from(request.query.limit).map_or(u32::MAX, |value| value);
        let hits = self
            .index
            .search_filtered(
                SparseSearchQuery {
                    vector,
                    limit,
                    max_contributions: 16,
                },
                &|chunk_id| self.chunk_allowed(chunk_id),
            )
            .map_err(port_error)?;
        let mut candidates = Vec::with_capacity(hits.len());
        let mut bytes_read = 0_u64;
        for hit in hits {
            let Some(candidate) = self.candidate_from_hit(hit)? else {
                continue;
            };
            let range = candidate.source_span.range();
            bytes_read = bytes_read.saturating_add(range.end.saturating_sub(range.start) as u64);
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
