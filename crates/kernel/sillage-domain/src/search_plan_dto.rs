//! Wire DTO for [`SearchPlan`] decode. Lives outside `search_plan.rs` so the
//! module stays within its size budget; decode reconstructs the domain type
//! exclusively through the validated builder.

use serde::Deserialize;

use super::search_plan::SearchPlan;
use crate::search::{
    CorpusScope, EvidenceRequirements, FreshnessRequirement, ModalitySet, SearchBudget,
    SearchCompatibilityError, SearchIntent, SearchRouteDecision, SearchStage, StopConditions,
};
use crate::{
    CorpusSnapshotId, IndexGenerationId, QueryId, RetrievalModelFingerprint,
    RetrievalPolicySnapshot,
};

#[derive(Deserialize)]
pub(crate) struct SearchPlanDto {
    pub(crate) query_id: QueryId,
    pub(crate) original_query: String,
    pub(crate) intent: SearchIntent,
    pub(crate) scope: CorpusScope,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) index_generation: IndexGenerationId,
    pub(crate) freshness: FreshnessRequirement,
    pub(crate) modalities: ModalitySet,
    pub(crate) stages: Vec<SearchStage>,
    pub(crate) budgets: SearchBudget,
    pub(crate) stop_conditions: StopConditions,
    pub(crate) evidence_requirements: EvidenceRequirements,
    pub(crate) fingerprint: RetrievalModelFingerprint,
    pub(crate) authorization: Option<RetrievalPolicySnapshot>,
    pub(crate) original_intent: Option<SearchIntent>,
    pub(crate) route_decision: Option<SearchRouteDecision>,
}

impl TryFrom<SearchPlanDto> for SearchPlan {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchPlanDto) -> Result<Self, Self::Error> {
        Self::builder()
            .query_id(dto.query_id)
            .original_query(dto.original_query)
            .intent(dto.intent)
            .scope(dto.scope)
            .corpus_snapshot(dto.corpus_snapshot)
            .index_generation(dto.index_generation)
            .freshness(dto.freshness)
            .modalities(dto.modalities)
            .stages(dto.stages)
            .budgets(dto.budgets)
            .stop_conditions(dto.stop_conditions)
            .evidence_requirements(dto.evidence_requirements)
            .fingerprint(dto.fingerprint)
            .authorization(
                dto.authorization
                    .ok_or(SearchCompatibilityError::InvalidPlan(
                        "authorization snapshot is required",
                    ))?,
            )
            .original_intent(dto.original_intent)
            .route_decision(dto.route_decision)
            .build()
    }
}
