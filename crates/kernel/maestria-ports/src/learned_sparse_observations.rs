use std::num::NonZeroUsize;

use maestria_domain::{
    ArtifactVersionId, CorpusSnapshotId, EvidenceId, EvidenceSpan, IndexGenerationId,
    LearnedSparseReason, QueryId, RetrievalLaneScore, RetrievalScoreKind, RetrievalScoreSet,
    SparseNamespace,
};
use serde::{Deserialize, Serialize};

use crate::SparseIdentity;

pub const LEARNED_SPARSE_SHADOW_SCHEMA_VERSION: u16 = 3;
pub const MAX_LEARNED_SPARSE_SHADOW_RETRIEVERS: usize = 8;
pub const MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS: usize = 256;
pub const MAX_LEARNED_SPARSE_SHADOW_ERROR_CHARS: usize = 512;
pub const MAX_LEARNED_SPARSE_SHADOW_LATENCY_MS: u64 = 5_000;
pub const MAX_LEARNED_SPARSE_SHADOW_CANDIDATES: usize = 256;
pub const MAX_LEARNED_SPARSE_SHADOW_CONTRIBUTIONS: usize = 64;
pub const MAX_LEARNED_SPARSE_SHADOW_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LearnedSparseQueryClass {
    ExactLiteral,
    VocabularyExpansion,
    DomainTerminology,
    MultiTerm,
    NoEvidence,
    Security,
}

impl LearnedSparseQueryClass {
    pub const fn all() -> [Self; 6] {
        [
            Self::ExactLiteral,
            Self::VocabularyExpansion,
            Self::DomainTerminology,
            Self::MultiTerm,
            Self::NoEvidence,
            Self::Security,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseShadowRoute {
    Shadow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseShadowCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub lane_rank: u32,
    pub score: RetrievalLaneScore,
    pub reason: LearnedSparseReason,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseShadowLaneStatus {
    Succeeded,
    Empty,
    Failed { error: String },
    PrivacyRejected,
    StaleGeneration,
    IncompatibleIdentity,
    BudgetExhausted,
    SecurityFiltered,
    TimedOut,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedSparseShadowLane {
    pub retriever_id: String,
    pub representation: maestria_domain::RepresentationName,
    pub generation: IndexGenerationId,
    #[serde(default)]
    pub namespace: Option<SparseNamespace>,
    #[serde(default)]
    pub sparse_identity: Option<SparseIdentity>,
    pub status: LearnedSparseShadowLaneStatus,
    pub candidates: Vec<LearnedSparseShadowCandidate>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedSparseShadowObservation {
    pub schema_version: u16,
    pub query_id: QueryId,
    pub query_class: LearnedSparseQueryClass,
    pub route: LearnedSparseShadowRoute,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub elapsed_ms: u64,
    pub lanes: Vec<LearnedSparseShadowLane>,
}

impl LearnedSparseShadowObservation {
    pub fn validate(&self) -> Result<(), LearnedSparseObservationValidationError> {
        if self.schema_version != LEARNED_SPARSE_SHADOW_SCHEMA_VERSION {
            return Err(LearnedSparseObservationValidationError::new(
                "unsupported schema version",
            ));
        }
        if self.elapsed_ms > MAX_LEARNED_SPARSE_SHADOW_LATENCY_MS {
            return Err(LearnedSparseObservationValidationError::new(
                "observation latency exceeds the bounded limit",
            ));
        }
        if self.lanes.len() > MAX_LEARNED_SPARSE_SHADOW_RETRIEVERS {
            return Err(LearnedSparseObservationValidationError::new(
                "retriever lane cap exceeded",
            ));
        }
        for lane in &self.lanes {
            let lane_namespace_valid = lane
                .namespace
                .as_ref()
                .is_some_and(|namespace| namespace.validate().is_ok());
            let lane_identity_valid = matches!(
                (&lane.namespace, &lane.sparse_identity),
                (Some(namespace), Some(identity))
                    if identity.validate().is_ok()
                        && identity.namespace == *namespace
                        && identity.corpus_snapshot == self.corpus_snapshot
                        && identity.generation_id == self.index_generation
                        && identity.generation_id == lane.generation
                        && identity.representation == lane.representation
            );
            if lane.retriever_id.trim().is_empty()
                || lane.generation != self.index_generation
                || !lane_namespace_valid
                || lane.candidates.len() > MAX_LEARNED_SPARSE_SHADOW_CANDIDATES
                || lane.candidates.iter().any(|candidate| {
                    candidate.score.score_kind != RetrievalScoreKind::LearnedSparse
                        || RetrievalScoreSet::single(candidate.score.clone()).is_err()
                        || candidate.reason.contributions.len()
                            > MAX_LEARNED_SPARSE_SHADOW_CONTRIBUTIONS
                })
                || (!lane.candidates.is_empty() && !lane_identity_valid)
            {
                return Err(LearnedSparseObservationValidationError::new(
                    "lane identity or candidate provenance is invalid",
                ));
            }
            if (matches!(lane.status, LearnedSparseShadowLaneStatus::Succeeded)
                && lane.candidates.is_empty())
                || (!matches!(lane.status, LearnedSparseShadowLaneStatus::Succeeded)
                    && !lane.candidates.is_empty())
                || lane
                    .candidates
                    .iter()
                    .any(|candidate| candidate.lane_rank == 0)
            {
                return Err(LearnedSparseObservationValidationError::new(
                    "lane status or candidate rank is invalid",
                ));
            }
            if let LearnedSparseShadowLaneStatus::Failed { error } = &lane.status
                && error.chars().count() > MAX_LEARNED_SPARSE_SHADOW_ERROR_CHARS
            {
                return Err(LearnedSparseObservationValidationError::new(
                    "failure reason exceeds the bounded error cap",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedSparseObservationValidationError(&'static str);

impl LearnedSparseObservationValidationError {
    const fn new(message: &'static str) -> Self {
        Self(message)
    }
}

impl std::fmt::Display for LearnedSparseObservationValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for LearnedSparseObservationValidationError {}

pub trait LearnedSparseObservationRepository: Send + Sync {
    fn append_observation(
        &self,
        observation: LearnedSparseShadowObservation,
    ) -> Result<(), super::PortError>;
    fn scan_observations(
        &self,
        limit: NonZeroUsize,
    ) -> Result<Vec<LearnedSparseShadowObservation>, super::PortError>;
    fn replace_observations(
        &self,
        observations: Vec<LearnedSparseShadowObservation>,
    ) -> Result<(), super::PortError>;
    fn prune_observations(&self, keep: NonZeroUsize) -> Result<(), super::PortError>;
}
