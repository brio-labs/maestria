use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{SearchBudget, SearchCompatibilityError, SearchIntent, search_plan_dto::SearchPlanDto};
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
    pub required_claims: Vec<String>,
    pub required_subquestions: Vec<String>,
    pub minimum_sources: usize,
    pub minimum_documents: usize,
    pub minimum_sections: usize,
}

/// Validated search plan: construction, mutation, and decode all enforce the
/// schema invariants in [`SearchPlan::validate_schema`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchPlanDto")]
pub struct SearchPlan {
    query_id: QueryId,
    original_query: String,
    intent: SearchIntent,
    scope: CorpusScope,
    corpus_snapshot: CorpusSnapshotId,
    index_generation: IndexGenerationId,
    freshness: FreshnessRequirement,
    modalities: ModalitySet,
    stages: Vec<SearchStage>,
    budgets: SearchBudget,
    stop_conditions: StopConditions,
    evidence_requirements: EvidenceRequirements,
    fingerprint: super::RetrievalModelFingerprint,
    /// Trusted request-bound authorization captured when the plan was created.
    authorization: crate::RetrievalPolicySnapshot,
    original_intent: Option<SearchIntent>,
    route_decision: Option<super::SearchRouteDecision>,
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
    pub fn builder() -> SearchPlanBuilder {
        SearchPlanBuilder::default()
    }

    pub const fn query_id(&self) -> QueryId {
        self.query_id
    }

    pub fn original_query(&self) -> &str {
        &self.original_query
    }

    pub const fn intent(&self) -> SearchIntent {
        self.intent
    }

    pub fn scope(&self) -> &CorpusScope {
        &self.scope
    }

    pub const fn corpus_snapshot(&self) -> CorpusSnapshotId {
        self.corpus_snapshot
    }

    pub const fn index_generation(&self) -> IndexGenerationId {
        self.index_generation
    }

    pub fn freshness(&self) -> &FreshnessRequirement {
        &self.freshness
    }

    pub fn modalities(&self) -> &ModalitySet {
        &self.modalities
    }

    pub fn stages(&self) -> &[SearchStage] {
        &self.stages
    }

    pub fn budgets(&self) -> &SearchBudget {
        &self.budgets
    }

    pub fn stop_conditions(&self) -> &StopConditions {
        &self.stop_conditions
    }

    pub fn evidence_requirements(&self) -> &EvidenceRequirements {
        &self.evidence_requirements
    }

    pub fn fingerprint(&self) -> &super::RetrievalModelFingerprint {
        &self.fingerprint
    }

    pub fn authorization(&self) -> &crate::RetrievalPolicySnapshot {
        &self.authorization
    }

    pub fn original_intent(&self) -> Option<SearchIntent> {
        self.original_intent
    }

    pub fn route_decision(&self) -> Option<&super::SearchRouteDecision> {
        self.route_decision.as_ref()
    }

    /// Returns a copy of this plan with a replaced original query.
    pub fn with_original_query(
        mut self,
        original_query: String,
    ) -> Result<Self, SearchCompatibilityError> {
        self.original_query = original_query;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with a replaced intent.
    pub fn with_intent(mut self, intent: SearchIntent) -> Result<Self, SearchCompatibilityError> {
        self.intent = intent;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with a replaced scope.
    pub fn with_scope(mut self, scope: CorpusScope) -> Result<Self, SearchCompatibilityError> {
        self.scope = scope;
        self.validate_schema()?;
        Ok(self)
    }

    /// Confines the plan to one instance scope (R43).
    ///
    /// A global plan is replaced with the restricted scope; a plan already
    /// restricted to exactly that scope is returned unchanged; a plan
    /// restricted to any other scope is rejected. This is the single typed
    /// transition used by every search surface so the runtime effect path and
    /// direct CLI/API searches enforce the same scope dimension.
    pub fn confine_to_scope(mut self, scope_id: ScopeId) -> Result<Self, SearchCompatibilityError> {
        match &self.scope {
            CorpusScope::Global => {
                self.scope = CorpusScope::Restricted(vec![scope_id]);
            }
            CorpusScope::Restricted(scopes) if scopes.as_slice() == [scope_id] => {}
            CorpusScope::Restricted(_scopes) => {
                return Err(SearchCompatibilityError::InvalidPlan(
                    "search plan is restricted to a scope outside the instance scope",
                ));
            }
        }
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with a replaced corpus snapshot.
    pub fn with_corpus_snapshot(
        mut self,
        corpus_snapshot: CorpusSnapshotId,
    ) -> Result<Self, SearchCompatibilityError> {
        self.corpus_snapshot = corpus_snapshot;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with a replaced freshness requirement.
    pub fn with_freshness(
        mut self,
        freshness: FreshnessRequirement,
    ) -> Result<Self, SearchCompatibilityError> {
        self.freshness = freshness;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with replaced modalities.
    pub fn with_modalities(
        mut self,
        modalities: ModalitySet,
    ) -> Result<Self, SearchCompatibilityError> {
        self.modalities = modalities;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with replaced stages.
    pub fn with_stages(
        mut self,
        stages: Vec<SearchStage>,
    ) -> Result<Self, SearchCompatibilityError> {
        self.stages = stages;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with replaced evidence requirements.
    pub fn with_evidence_requirements(
        mut self,
        evidence_requirements: EvidenceRequirements,
    ) -> Result<Self, SearchCompatibilityError> {
        self.evidence_requirements = evidence_requirements;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with replaced stop conditions.
    pub fn with_stop_conditions(
        mut self,
        stop_conditions: StopConditions,
    ) -> Result<Self, SearchCompatibilityError> {
        self.stop_conditions = stop_conditions;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with replaced budgets.
    pub fn with_budgets(mut self, budgets: SearchBudget) -> Result<Self, SearchCompatibilityError> {
        self.budgets = budgets;
        self.validate_schema()?;
        Ok(self)
    }

    /// Returns a copy of this plan with a replaced authorization snapshot.
    pub fn with_authorization(
        mut self,
        authorization: crate::RetrievalPolicySnapshot,
    ) -> Result<Self, SearchCompatibilityError> {
        self.authorization = authorization;
        self.validate_schema()?;
        Ok(self)
    }

    /// Validates schema invariants before policy or runtime evaluation.
    pub fn validate_schema(&self) -> Result<(), SearchCompatibilityError> {
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

#[path = "search_plan_builder.rs"]
mod search_plan_builder;
pub use search_plan_builder::SearchPlanBuilder;
