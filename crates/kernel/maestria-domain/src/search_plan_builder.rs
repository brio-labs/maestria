use super::{SearchCompatibilityError, SearchPlan, SearchStage};
use crate::ids::{CorpusSnapshotId, IndexGenerationId, QueryId};
use crate::search::{
    CorpusScope, EvidenceRequirements, FreshnessRequirement, ModalitySet, SearchBudget,
    SearchIntent, StopConditions,
};

/// Incremental constructor for [`SearchPlan`]; [`build`](SearchPlanBuilder::build)
/// assembles the plan and enforces every schema invariant.
#[derive(Default)]
pub struct SearchPlanBuilder {
    query_id: Option<QueryId>,
    original_query: Option<String>,
    intent: Option<SearchIntent>,
    scope: Option<CorpusScope>,
    corpus_snapshot: Option<CorpusSnapshotId>,
    index_generation: Option<IndexGenerationId>,
    freshness: Option<FreshnessRequirement>,
    modalities: Option<ModalitySet>,
    stages: Option<Vec<SearchStage>>,
    budgets: Option<SearchBudget>,
    stop_conditions: Option<StopConditions>,
    evidence_requirements: Option<EvidenceRequirements>,
    fingerprint: Option<crate::RetrievalModelFingerprint>,
    authorization: Option<Option<crate::RetrievalPolicySnapshot>>,
    original_intent: Option<Option<SearchIntent>>,
    route_decision: Option<Option<String>>,
}

impl SearchPlanBuilder {
    pub fn query_id(mut self, value: QueryId) -> Self {
        self.query_id = Some(value);
        self
    }

    pub fn original_query(mut self, value: String) -> Self {
        self.original_query = Some(value);
        self
    }

    pub fn intent(mut self, value: SearchIntent) -> Self {
        self.intent = Some(value);
        self
    }

    pub fn scope(mut self, value: CorpusScope) -> Self {
        self.scope = Some(value);
        self
    }

    pub fn corpus_snapshot(mut self, value: CorpusSnapshotId) -> Self {
        self.corpus_snapshot = Some(value);
        self
    }

    pub fn index_generation(mut self, value: IndexGenerationId) -> Self {
        self.index_generation = Some(value);
        self
    }

    pub fn freshness(mut self, value: FreshnessRequirement) -> Self {
        self.freshness = Some(value);
        self
    }

    pub fn modalities(mut self, value: ModalitySet) -> Self {
        self.modalities = Some(value);
        self
    }

    pub fn stages(mut self, value: Vec<SearchStage>) -> Self {
        self.stages = Some(value);
        self
    }

    pub fn budgets(mut self, value: SearchBudget) -> Self {
        self.budgets = Some(value);
        self
    }

    pub fn stop_conditions(mut self, value: StopConditions) -> Self {
        self.stop_conditions = Some(value);
        self
    }

    pub fn evidence_requirements(mut self, value: EvidenceRequirements) -> Self {
        self.evidence_requirements = Some(value);
        self
    }

    pub fn fingerprint(mut self, value: crate::RetrievalModelFingerprint) -> Self {
        self.fingerprint = Some(value);
        self
    }

    pub fn authorization(mut self, value: Option<crate::RetrievalPolicySnapshot>) -> Self {
        self.authorization = Some(value);
        self
    }

    pub fn original_intent(mut self, value: Option<SearchIntent>) -> Self {
        self.original_intent = Some(value);
        self
    }

    pub fn route_decision(mut self, value: Option<String>) -> Self {
        self.route_decision = Some(value);
        self
    }

    /// Assembles the plan, failing when a required field is missing or the
    /// resulting plan violates a schema invariant.
    pub fn build(self) -> Result<SearchPlan, SearchCompatibilityError> {
        let plan = SearchPlan {
            query_id: required(self.query_id)?,
            original_query: required(self.original_query)?,
            intent: required(self.intent)?,
            scope: required(self.scope)?,
            corpus_snapshot: required(self.corpus_snapshot)?,
            index_generation: required(self.index_generation)?,
            freshness: required(self.freshness)?,
            modalities: required(self.modalities)?,
            stages: required(self.stages)?,
            budgets: required(self.budgets)?,
            stop_conditions: required(self.stop_conditions)?,
            evidence_requirements: required(self.evidence_requirements)?,
            fingerprint: required(self.fingerprint)?,
            authorization: self.authorization.flatten(),
            original_intent: self.original_intent.flatten(),
            route_decision: self.route_decision.flatten(),
        };
        plan.validate_schema()?;
        Ok(plan)
    }
}

fn required<T>(value: Option<T>) -> Result<T, SearchCompatibilityError> {
    value.ok_or(SearchCompatibilityError::InvalidPlan(
        "search plan is missing a required field",
    ))
}
