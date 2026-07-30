use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{SearchBudget, SearchCompatibilityError, SearchIntent};
use crate::ids::{CorpusSnapshotId, IndexGenerationId, QueryId, ScopeId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorpusScope {
    Global,
    Restricted(Vec<ScopeId>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreshnessRequirement {
    Any,
    Realtime,
    MaximumAgeDays(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Modality {
    Text,
    Image,
    Code,
    Pdf,
    Table,
    Web,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ModalitySetDto")]
pub struct ModalitySet {
    values: Vec<Modality>,
}

#[derive(Deserialize)]
struct ModalitySetDto {
    values: Vec<Modality>,
}

impl TryFrom<ModalitySetDto> for ModalitySet {
    type Error = SearchCompatibilityError;

    fn try_from(dto: ModalitySetDto) -> Result<Self, Self::Error> {
        let mut values = dto.values;
        values.sort();
        values.dedup();
        Ok(Self { values })
    }
}

impl ModalitySet {
    pub fn new(values: Vec<Modality>) -> Self {
        let mut values = values;
        values.sort();
        values.dedup();
        Self { values }
    }

    pub fn values(&self) -> &[Modality] {
        &self.values
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SearchStage {
    InitialRetrieval,
    Reranking,
    Filtering,
    Synthesis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopConditions {
    pub max_results: u32,
    pub min_score_threshold: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRequirements {
    pub require_primary_sources: bool,
    pub minimum_corroboration: u8,
    #[serde(default)]
    pub required_claims: Vec<String>,
    #[serde(default)]
    pub required_subquestions: Vec<String>,
    #[serde(default)]
    pub minimum_sources: usize,
    #[serde(default)]
    pub minimum_documents: usize,
    #[serde(default)]
    pub minimum_sections: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPlan {
    pub query_id: QueryId,
    pub original_query: String,
    pub intent: SearchIntent,
    pub scope: CorpusScope,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub freshness: FreshnessRequirement,
    pub modalities: ModalitySet,
    pub stages: Vec<SearchStage>,
    pub budgets: SearchBudget,
    pub stop_conditions: StopConditions,
    pub evidence_requirements: EvidenceRequirements,
    pub fingerprint: super::RetrievalModelFingerprint,
    /// Trusted request-bound authorization captured when the plan was created.
    /// Missing snapshots represent legacy plans and are rejected on execution.
    #[serde(default)]
    pub authorization: Option<crate::RetrievalPolicySnapshot>,
    pub original_intent: Option<SearchIntent>,
    pub route_decision: Option<String>,
}

fn validate_web_budget(plan: &SearchPlan) -> Result<(), SearchCompatibilityError> {
    if plan.intent == SearchIntent::CurrentWeb || plan.modalities.values().contains(&Modality::Web)
    {
        if plan.budgets.max_web_requests() == 0 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "web plans require a positive web request budget",
            ));
        }
        if plan.budgets.max_bytes_read() == 0 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "web plans require a positive byte budget",
            ));
        }
    }
    Ok(())
}

impl SearchPlan {
    /// Validates schema invariants before policy or runtime evaluation.
    pub fn validate_schema(&self) -> Result<(), SearchCompatibilityError> {
        if self.authorization.is_none() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "authorization snapshot is required",
            ));
        }
        if self.original_query.trim().is_empty() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "original_query must not be empty",
            ));
        }
        let query_tokens = self.original_query.split_whitespace().count().max(1);
        if query_tokens > self.budgets.max_tokens() as usize {
            return Err(SearchCompatibilityError::InvalidPlan(
                "original_query exceeds the token budget",
            ));
        }
        if self.modalities.values().is_empty() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "at least one modality is required",
            ));
        }
        if self.stages.is_empty() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "at least one search stage is required",
            ));
        }
        if self.stages[0] != SearchStage::InitialRetrieval {
            return Err(SearchCompatibilityError::InvalidPlan(
                "initial retrieval must be the first stage",
            ));
        }
        let unique_stages = self.stages.iter().collect::<BTreeSet<_>>();
        if unique_stages.len() != self.stages.len() {
            return Err(SearchCompatibilityError::InvalidPlan(
                "search stages must not repeat",
            ));
        }
        if self.stages.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(SearchCompatibilityError::InvalidPlan(
                "search stages must use canonical execution order",
            ));
        }
        if self.stages.len() > self.budgets.max_stages() as usize {
            return Err(SearchCompatibilityError::InvalidPlan(
                "search stages exceed the stage budget",
            ));
        }
        if self.stop_conditions.max_results == 0 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "max_results must be greater than 0",
            ));
        }
        if self.stop_conditions.min_score_threshold > 10_000 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "min_score_threshold must be between 0 and 10000",
            ));
        }
        if let CorpusScope::Restricted(scopes) = &self.scope {
            if scopes.is_empty() {
                return Err(SearchCompatibilityError::InvalidPlan(
                    "restricted scope must contain at least one scope",
                ));
            }
            let unique_scopes = scopes.iter().collect::<BTreeSet<_>>();
            if unique_scopes.len() != scopes.len() {
                return Err(SearchCompatibilityError::InvalidPlan(
                    "restricted scope identifiers must not repeat",
                ));
            }
        }
        if matches!(self.freshness, FreshnessRequirement::MaximumAgeDays(0)) {
            return Err(SearchCompatibilityError::InvalidPlan(
                "maximum freshness age must be greater than 0 days",
            ));
        }
        validate_web_budget(self)?;
        if self.evidence_requirements.minimum_corroboration == 0 {
            return Err(SearchCompatibilityError::InvalidPlan(
                "minimum corroboration must be greater than 0",
            ));
        }
        if self
            .evidence_requirements
            .required_claims
            .iter()
            .chain(self.evidence_requirements.required_subquestions.iter())
            .any(|value| value.trim().is_empty())
        {
            return Err(SearchCompatibilityError::InvalidPlan(
                "required claims and subquestions must not be empty",
            ));
        }
        Ok(())
    }
    pub fn execution_budget(
        &self,
    ) -> Result<super::SearchExecutionBudget, SearchCompatibilityError> {
        self.budgets
            .execution_budget(self.stop_conditions.max_results)
    }
}
