use maestria_domain::{EvidenceCandidate, SearchLaneStatus, SearchOutcome, SearchPlan};
use maestria_ports::SearchQuery;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrieverDescriptor {
    pub id: String,
    pub modality: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRequest {
    pub plan: SearchPlan,
    pub query: SearchQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBatch {
    pub descriptor: RetrieverDescriptor,
    pub candidates: Vec<EvidenceCandidate>,
    pub status: SearchLaneStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusedCandidate {
    pub candidate: EvidenceCandidate,
    pub fused_score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedCandidate {
    pub candidate: EvidenceCandidate,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankRequest {
    pub plan: SearchPlan,
    pub candidates: Vec<RankedCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankResult {
    pub candidates: Vec<RankedCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionPolicy {
    pub max_results: usize,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalExperiment {
    pub plan: SearchPlan,
    pub candidates: Vec<EvidenceCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalEvaluationReport {
    pub outcome: SearchOutcome,
    pub evaluated_candidates: usize,
}

#[derive(Error, Debug)]
pub enum RetrievalError {
    #[error("Compatibility error: {0}")]
    Compatibility(#[from] maestria_domain::SearchCompatibilityError),
    #[error("Retrieval cancelled")]
    Cancelled,
    #[error("Retrieval timed out")]
    Timeout,
    #[error("Internal engine error: {0}")]
    Internal(String),
}

pub type RetrievalResult<T> = Result<T, RetrievalError>;
