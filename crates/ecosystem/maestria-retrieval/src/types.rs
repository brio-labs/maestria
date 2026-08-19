use maestria_domain::{
    ArtifactId, EvidenceCandidate, IndexGenerationId, RepresentationName, SearchExecution,
    SearchExecutionBudget, SearchLaneStatus, SearchOutcome, SearchPlan,
};
use maestria_ports::SearchQuery;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrieverDescriptor {
    pub id: String,
    pub modality: String,
    pub representation: RepresentationName,
    pub generation: IndexGenerationId,
}

impl RetrieverDescriptor {
    pub fn is_code(&self) -> bool {
        self.modality.eq_ignore_ascii_case("code")
            || self.modality.eq_ignore_ascii_case("rust")
            || self.id.to_ascii_lowercase().contains("code_intel")
    }

    pub fn is_dense(&self) -> bool {
        let id = self.id.to_ascii_lowercase();
        id.contains("dense") || id.contains("vector") || id.contains("semantic")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CandidateSourceFilterError {
    #[error("source filter must contain at least one artifact")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSourceFilter {
    allowed_artifact_ids: BTreeSet<ArtifactId>,
}

impl CandidateSourceFilter {
    pub fn try_new(
        allowed_artifact_ids: BTreeSet<ArtifactId>,
    ) -> Result<Self, CandidateSourceFilterError> {
        if allowed_artifact_ids.is_empty() {
            return Err(CandidateSourceFilterError::Empty);
        }
        Ok(Self {
            allowed_artifact_ids,
        })
    }

    pub fn allows(&self, artifact_id: ArtifactId) -> bool {
        self.allowed_artifact_ids.contains(&artifact_id)
    }

    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"maestria:candidate-source-filter:v1\0");
        for artifact_id in &self.allowed_artifact_ids {
            hasher.update(artifact_id.value().to_be_bytes());
        }
        let digest = hasher.finalize();
        let mut output = String::with_capacity(71);
        output.push_str("sha256:");
        for byte in digest {
            output.push_str(&format!("{byte:02x}"));
        }
        output
    }

    pub fn artifact_ids(&self) -> &BTreeSet<ArtifactId> {
        &self.allowed_artifact_ids
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRequest {
    pub plan: std::sync::Arc<SearchPlan>,
    pub query: SearchQuery,
    pub execution_budget: SearchExecutionBudget,
    pub expected_generation: IndexGenerationId,
    pub authorization: maestria_governance::RetrievalAuthorizationContext,
    pub source_filter: Option<CandidateSourceFilter>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBatch {
    pub descriptor: RetrieverDescriptor,
    pub query: String,
    pub candidates: Vec<EvidenceCandidate>,
    pub status: SearchLaneStatus,
    pub generation: Option<IndexGenerationId>,
    pub execution: SearchExecution,
}

impl CandidateBatch {
    pub fn succeeded(
        descriptor: RetrieverDescriptor,
        query: String,
        candidates: Vec<EvidenceCandidate>,
        generation: Option<IndexGenerationId>,
        execution: SearchExecution,
    ) -> Self {
        Self {
            descriptor,
            query,
            candidates,
            status: SearchLaneStatus::Succeeded,
            generation,
            execution,
        }
    }

    pub fn failed(
        descriptor: RetrieverDescriptor,
        query: String,
        error: String,
        generation: Option<IndexGenerationId>,
        execution: SearchExecution,
    ) -> Self {
        Self {
            descriptor,
            query,
            candidates: Vec::new(),
            status: SearchLaneStatus::Failed { error },
            generation,
            execution,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusedCandidate {
    pub candidate: EvidenceCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedCandidate {
    pub candidate: EvidenceCandidate,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HybridPromotionRecord {
    evaluation_id: String,
    evaluation_date: String,
    /// Query classes served by the dense lane; all other classes keep the
    /// lexical route (per-class protection mirrors the sparse policy).
    served_classes: std::collections::BTreeSet<crate::LearnedSparseQueryClass>,
}

impl HybridPromotionRecord {
    pub fn new(
        evaluation_id: String,
        evaluation_date: String,
        served_classes: std::collections::BTreeSet<crate::LearnedSparseQueryClass>,
    ) -> Option<Self> {
        (!evaluation_id.trim().is_empty() && !evaluation_date.trim().is_empty()).then_some(Self {
            evaluation_id,
            evaluation_date,
            served_classes,
        })
    }

    pub fn serves_class(&self, class: &crate::LearnedSparseQueryClass) -> bool {
        self.served_classes.contains(class)
    }

    pub fn evaluation_id(&self) -> &str {
        &self.evaluation_id
    }

    pub fn evaluation_date(&self) -> &str {
        &self.evaluation_date
    }

    pub fn served_classes(&self) -> &std::collections::BTreeSet<crate::LearnedSparseQueryClass> {
        &self.served_classes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HybridExecutionPolicy {
    #[default]
    Shadow,
    Active(HybridPromotionRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankRequest {
    pub plan: std::sync::Arc<SearchPlan>,
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
    pub plan: Arc<SearchPlan>,
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
    pub authorization: maestria_governance::RetrievalAuthorizationContext,
    pub execution_budget: SearchExecutionBudget,
    pub source_filter: Option<CandidateSourceFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextExpansion {
    pub candidates: Vec<EvidenceCandidate>,
    pub execution: SearchExecution,
}
pub struct RetrievalExperiment {
    pub plan: std::sync::Arc<SearchPlan>,
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
    #[error("result limit {limit} exceeds the supported u32 maximum")]
    InvalidResultLimit { limit: usize },
    #[error("Retrieval cancelled")]
    Cancelled,
    #[error("Retrieval timed out")]
    Timeout,
    #[error("artifact {artifact_id} has no immutable content-addressed version")]
    MissingArtifactVersion {
        artifact_id: maestria_domain::ArtifactId,
    },
    #[error("repository code index error: {0}")]
    CodeIntel(#[from] maestria_code_intel::CodeIntelError),
    #[error("Internal engine error: {0}")]
    Internal(String),
}

pub type RetrievalResult<T> = Result<T, RetrievalError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounded_expansion_inputs() -> Result<(), Box<dyn std::error::Error>> {
        let policy = ExpansionPolicy {
            max_results: 5,
            max_depth: 2,
            selected_seeds: vec![],
            required_claims: vec!["claim".to_string()],
            required_subquestions: vec![],
            authorization: maestria_governance::RetrievalSecurityPolicy::default()
                .authorization_context(&maestria_domain::CorpusScope::Global)?,
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 5, 5, 0)?,
            source_filter: None,
        };
        assert_eq!(policy.max_results, 5);
        assert_eq!(policy.required_claims.len(), 1);
        Ok(())
    }
}
