use std::sync::Arc;

use super::SourceSnapshotVerifier;
use super::common::{generation_mismatch, one_based_rank, port_error};
use super::learned_sparse_generation::LearnedSparseGenerationCapability;
use super::prescore_cache::PrescoreCache;
use super::sparse_record_cache::RecordCache;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};
use async_trait::async_trait;
use maestria_domain::{RetrievalModelFingerprint, SearchLaneStatus};
use maestria_governance::scan_secrets;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, LearnedSparseIndex,
    LearnedSparseProvider, RetentionPolicy, SparseIdentity, SparseInputKind, SparseSearchQuery,
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
    pub(super) index: Arc<dyn LearnedSparseIndex + Send + Sync>,
    pub(super) artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub(super) chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub(super) evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub(super) provider: Arc<dyn LearnedSparseProvider + Send + Sync>,
    pub(super) verifier: SourceSnapshotVerifier,
    pub(super) identity: SparseIdentity,
    pub(super) fingerprint: RetrievalModelFingerprint,
    pub(super) descriptor: RetrieverDescriptor,
}
pub(super) type SparseRecords = (
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
        let prescore_cache = PrescoreCache::new(std::cmp::max(
            request.query.limit,
            usize::try_from(request.execution_budget.max_candidates())
                .map_or(usize::from(u16::MAX), usize::from),
        ));
        let record_cache = RecordCache::new();
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
                    self.preflight_chunk(chunk_id, &request, &prescore_cache, &record_cache)
                        .map_err(|error| maestria_ports::PortError::InternalContext {
                            context: "sparse authorization filter",
                            source: error.to_string(),
                        })
                },
            )
            .map_err(port_error)?;
        let hits = bounded.hits;
        let score_context = super::score_provenance::LearnedSparseScoreContext::new(
            &self.identity,
            self.fingerprint.clone(),
        );
        let assembly = super::sparse_records::SparseAssembly {
            source_filter: request.source_filter.as_ref(),
            score: &score_context,
        };
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
                &assembly,
                &prescore_cache,
                &record_cache,
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
