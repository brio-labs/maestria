use std::sync::Arc;

use super::SourceSnapshotVerifier;
use super::chunk_access::{load_authorized_chunk, source_filter_allows_chunk};
use super::common::{candidate_from_records, generation_mismatch, one_based_rank, port_error};
use super::learned_sparse_generation::LearnedSparseGenerationCapability;
use super::prescore_cache::PrescoreCache;
use super::score_provenance::learned_sparse_score;
use crate::traits::CandidateRetriever;
use crate::types::{
    CandidateBatch, CandidateRequest, CandidateSourceFilter, RetrievalError, RetrieverDescriptor,
};
use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, LearnedSparseContribution, LearnedSparseReason, RetrievalModelFingerprint,
    RetrievalReason, SearchLaneStatus,
};
use maestria_governance::scan_secrets;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, LearnedSparseIndex,
    LearnedSparseProvider, RetentionPolicy, SparseIdentity, SparseInputKind, SparseSearchHit,
    SparseSearchQuery,
};

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
    identity: SparseIdentity,
    fingerprint: RetrievalModelFingerprint,
    descriptor: RetrieverDescriptor,
}
type SparseRecords = (
    maestria_domain::Artifact,
    maestria_domain::Chunk,
    maestria_domain::Evidence,
);

impl LearnedSparseChunkRetriever {
    pub fn new(
        parts: LearnedSparseChunkRetrieverParts,
        capability: LearnedSparseGenerationCapability,
    ) -> Result<Self, RetrievalError> {
        let identity = capability.identity().clone();
        let serving_eligible = capability.is_serving_eligible();
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
            modality: if serving_eligible {
                "sparse".to_string()
            } else {
                "sparse-shadow".to_string()
            },
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
        if request.plan.corpus_snapshot() != self.identity.corpus_snapshot {
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
    fn checked_records(
        &self,
        chunk_id: maestria_domain::ChunkId,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
    ) -> Result<
        Option<(
            maestria_domain::Artifact,
            maestria_domain::Chunk,
            maestria_domain::Evidence,
        )>,
        RetrievalError,
    > {
        if !source_filter_allows_chunk(self.chunks.as_ref(), chunk_id, source_filter)
            .map_err(port_error)?
        {
            return Ok(None);
        }
        let Some((artifact, chunk)) = load_authorized_chunk(
            self.chunks.as_ref(),
            self.artifacts.as_ref(),
            chunk_id,
            authorization,
        )
        .map_err(port_error)?
        else {
            return Ok(None);
        };
        let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
        let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
            return Ok(None);
        };
        if evidence.artifact_id != artifact.id {
            return Err(port_error(maestria_ports::PortError::Conflict {
                message: format!(
                    "evidence {} owner mismatch: expected artifact {}, got {}",
                    evidence.id, artifact.id, evidence.artifact_id
                ),
            }));
        }
        if authorization.evaluate(&evidence.security)
            != maestria_governance::RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Ok(None);
        }
        self.verifier.verify(&evidence, &artifact)?;
        Ok(Some((artifact, chunk, evidence)))
    }

    fn candidate_from_hit(
        &self,
        hit: SparseSearchHit,
        raw_rank: u32,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<&CandidateSourceFilter>,
        prescore_cache: &PrescoreCache<SparseRecords>,
    ) -> Result<Option<EvidenceCandidate>, RetrievalError> {
        let records = match prescore_cache.take(hit.chunk_id) {
            Some(records) => Some(records),
            None => self.checked_records(hit.chunk_id, authorization, source_filter)?,
        };
        let Some((artifact, chunk, evidence)) = records else {
            return Ok(None);
        };
        if source_filter.is_some_and(|filter| !filter.allows(artifact.id)) {
            return Ok(None);
        }
        let contributions = hit
            .contributions
            .into_iter()
            .map(|contribution| LearnedSparseContribution {
                term_id: contribution.term_id,
                contribution_micros: contribution.contribution_micros,
            })
            .collect();
        candidate_from_records(
            artifact.id,
            artifact.content_hash.as_ref(),
            &chunk.source_span,
            &evidence,
            chunk.node_id,
            learned_sparse_score(
                &self.identity,
                self.fingerprint.clone(),
                hit.score_micros,
                raw_rank,
            )?,
            vec![RetrievalReason::LearnedSparse(Box::new(
                LearnedSparseReason::new(contributions),
            ))],
        )
        .map(Some)
    }
    fn preflight_chunk(
        &self,
        chunk_id: maestria_domain::ChunkId,
        request: &CandidateRequest,
        prescore_cache: &PrescoreCache<SparseRecords>,
    ) -> Result<bool, RetrievalError> {
        let Some(records) = self.checked_records(
            chunk_id,
            &request.authorization,
            request.source_filter.as_ref(),
        )?
        else {
            return Ok(false);
        };
        if request
            .source_filter
            .as_ref()
            .is_some_and(|filter| !filter.allows(records.0.id))
        {
            return Ok(false);
        }
        prescore_cache.insert(chunk_id, records);
        Ok(true)
    }
}

#[async_trait]
impl CandidateRetriever for LearnedSparseChunkRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }
    fn sparse_namespace(&self) -> Option<maestria_domain::SparseNamespace> {
        Some(self.identity.namespace.clone())
    }

    fn sparse_identity(&self) -> Option<SparseIdentity> {
        Some(self.identity.clone())
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
        let limit = u32::try_from(request.query.limit).map_err(|_| {
            RetrievalError::Internal("sparse result limit exceeds query representation".into())
        })?;
        let prescore_cache = PrescoreCache::new(request.query.limit);
        let bounded = self
            .index
            .search_filtered(
                SparseSearchQuery {
                    vector,
                    limit,
                    max_contributions: maestria_domain::saturating_u32(
                        maestria_domain::saturating_usize(
                            request.execution_budget.max_work_units(),
                        ),
                    ),
                    execution_budget: request.execution_budget,
                },
                &|chunk_id| {
                    self.preflight_chunk(chunk_id, &request, &prescore_cache)
                        .map_err(|error| maestria_ports::PortError::InternalContext {
                            context: "sparse authorization filter",
                            source: error.to_string(),
                        })
                },
            )
            .map_err(port_error)?;
        let hits = bounded.hits;
        let mut candidates = Vec::with_capacity(hits.len().min(maestria_domain::saturating_usize(
            request.execution_budget.max_results(),
        )));
        for (raw_rank, hit) in hits
            .into_iter()
            .take(maestria_domain::saturating_usize(
                request.execution_budget.max_results(),
            ))
            .enumerate()
        {
            let Some(candidate) = self.candidate_from_hit(
                hit,
                one_based_rank(raw_rank)?,
                &request.authorization,
                request.source_filter.as_ref(),
                &prescore_cache,
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
}
