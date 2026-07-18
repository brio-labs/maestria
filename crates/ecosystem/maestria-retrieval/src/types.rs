use maestria_domain::{
    EvidenceCandidate, IndexGenerationId, RepresentationName, SearchLaneStatus, SearchOutcome,
    SearchPlan,
};
use maestria_ports::SearchQuery;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrieverDescriptor {
    pub id: String,
    pub modality: String,
    pub representation: RepresentationName,
    pub generation: IndexGenerationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRequest {
    pub plan: SearchPlan,
    pub query: SearchQuery,
    pub expected_generation: IndexGenerationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBatch {
    pub descriptor: RetrieverDescriptor,
    pub query: String,
    pub candidates: Vec<EvidenceCandidate>,
    pub status: SearchLaneStatus,
    pub generation: Option<IndexGenerationId>,
    pub bytes_read: u64,
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
pub struct HybridPromotionRecord {
    evaluation_id: String,
    evaluation_date: String,
}

impl HybridPromotionRecord {
    pub fn new(evaluation_id: String, evaluation_date: String) -> Option<Self> {
        (!evaluation_id.trim().is_empty() && !evaluation_date.trim().is_empty()).then_some(Self {
            evaluation_id,
            evaluation_date,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HybridExecutionPolicy {
    #[default]
    Shadow,
    Active(HybridPromotionRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalMode {
    LexicalOnly,
    HybridShadow,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankRequest {
    pub plan: SearchPlan,
    pub candidates: Vec<RankedCandidate>,
    pub max_latency_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankResult {
    pub candidates: Vec<RankedCandidate>,
    pub trace: maestria_domain::SearchTraceRerank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankScoreComponents {
    pub relevance: u32,
    pub constraints: Vec<RerankConstraintScore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankConstraintScore {
    pub name: String,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankScorerInput {
    pub plan: SearchPlan,
    pub candidate: EvidenceCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankLimits {
    pub input_cap: usize,
    pub score_cap: usize,
    pub output_cap: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionPolicy {
    pub max_results: usize,
    pub max_depth: usize,
    pub selected_seeds: Vec<maestria_domain::EvidenceCandidate>,
    pub required_claims: Vec<String>,
    pub required_subquestions: Vec<String>,
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
    #[error("Search plan rejected: {0}")]
    SearchPlan(#[from] maestria_governance::SearchPlanValidationError),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounded_expansion_inputs() {
        let policy = ExpansionPolicy {
            max_results: 5,
            max_depth: 2,
            selected_seeds: vec![],
            required_claims: vec!["claim".to_string()],
            required_subquestions: vec![],
        };
        assert_eq!(policy.max_results, 5);
        assert_eq!(policy.required_claims.len(), 1);
    }
}
