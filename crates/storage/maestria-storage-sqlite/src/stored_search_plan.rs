//! DTO mirrors of the maestria-domain search *plan* core.
//!
//! The stored row owns its own wire format: every `Stored*` type here is a
//! serde shape independent of `maestria_domain`, with infallible
//! `from_domain` encoding and validated, fallible `try_into_domain` decoding.
//! No legacy wire shapes are preserved. The plan-side types are re-exported
//! from `crate::payloads::stored_search` alongside the outcome-side types in
//! [`crate::payloads::stored_search_outcome`].
//!
//! This module is a façade: the budget / model-fingerprint / policy-snapshot
//! types live in [`crate::payloads::stored_search_plan_policy`] and the
//! stop-conditions / evidence-requirements types in
//! [`crate::payloads::stored_search_plan_requirements`]; both are re-exported
//! here so existing import paths keep working unchanged.

use maestria_domain::{
    CorpusScope, FreshnessRequirement, IndexGenerationId, Modality, ModalitySet, QueryId, ScopeId,
    SearchIntent, SearchPlan, SearchStage,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

pub(crate) use crate::payloads::stored_search_plan_policy::{
    StoredRetrievalModelFingerprint, StoredRetrievalPolicySnapshot, StoredSearchBudget,
};
pub(crate) use crate::payloads::stored_search_plan_requirements::{
    StoredEvidenceRequirements, StoredStopConditions,
};
pub(crate) use crate::payloads::stored_search_route::StoredSearchRouteDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchIntent {
    ExactLookup,
    FactualLocal,
    SemanticDiscovery,
    CompositionalConstraints,
    MultiHop,
    CorpusSynthesis,
    RepositoryCode,
    VisualDocument,
    TemporalMemory,
    CurrentWeb,
    ContradictionAudit,
}

impl StoredSearchIntent {
    pub(crate) fn from_domain(value: &SearchIntent) -> Self {
        match value {
            SearchIntent::ExactLookup => Self::ExactLookup,
            SearchIntent::FactualLocal => Self::FactualLocal,
            SearchIntent::SemanticDiscovery => Self::SemanticDiscovery,
            SearchIntent::CompositionalConstraints => Self::CompositionalConstraints,
            SearchIntent::MultiHop => Self::MultiHop,
            SearchIntent::CorpusSynthesis => Self::CorpusSynthesis,
            SearchIntent::RepositoryCode => Self::RepositoryCode,
            SearchIntent::VisualDocument => Self::VisualDocument,
            SearchIntent::TemporalMemory => Self::TemporalMemory,
            SearchIntent::CurrentWeb => Self::CurrentWeb,
            SearchIntent::ContradictionAudit => Self::ContradictionAudit,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchIntent, PortError> {
        Ok(match self {
            Self::ExactLookup => SearchIntent::ExactLookup,
            Self::FactualLocal => SearchIntent::FactualLocal,
            Self::SemanticDiscovery => SearchIntent::SemanticDiscovery,
            Self::CompositionalConstraints => SearchIntent::CompositionalConstraints,
            Self::MultiHop => SearchIntent::MultiHop,
            Self::CorpusSynthesis => SearchIntent::CorpusSynthesis,
            Self::RepositoryCode => SearchIntent::RepositoryCode,
            Self::VisualDocument => SearchIntent::VisualDocument,
            Self::TemporalMemory => SearchIntent::TemporalMemory,
            Self::CurrentWeb => SearchIntent::CurrentWeb,
            Self::ContradictionAudit => SearchIntent::ContradictionAudit,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredCorpusScope {
    Global,
    Restricted(Vec<u64>),
}

impl StoredCorpusScope {
    pub(crate) fn from_domain(value: &CorpusScope) -> Self {
        match value {
            CorpusScope::Global => Self::Global,
            CorpusScope::Restricted(scopes) => {
                Self::Restricted(scopes.iter().map(ScopeId::value).collect())
            }
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<CorpusScope, PortError> {
        Ok(match self {
            Self::Global => CorpusScope::Global,
            Self::Restricted(scopes) => {
                CorpusScope::Restricted(scopes.into_iter().map(ScopeId::new).collect())
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredFreshnessRequirement {
    Any,
    Realtime,
    MaximumAgeDays(u32),
}

impl StoredFreshnessRequirement {
    pub(crate) fn from_domain(value: &FreshnessRequirement) -> Self {
        match value {
            FreshnessRequirement::Any => Self::Any,
            FreshnessRequirement::Realtime => Self::Realtime,
            FreshnessRequirement::MaximumAgeDays(days) => Self::MaximumAgeDays(*days),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<FreshnessRequirement, PortError> {
        Ok(match self {
            Self::Any => FreshnessRequirement::Any,
            Self::Realtime => FreshnessRequirement::Realtime,
            Self::MaximumAgeDays(days) => FreshnessRequirement::MaximumAgeDays(days),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredModality {
    Text,
    Image,
    Code,
    Pdf,
    Table,
    Web,
    Command,
}

impl StoredModality {
    pub(crate) fn from_domain(value: &Modality) -> Self {
        match value {
            Modality::Text => Self::Text,
            Modality::Image => Self::Image,
            Modality::Code => Self::Code,
            Modality::Pdf => Self::Pdf,
            Modality::Table => Self::Table,
            Modality::Web => Self::Web,
            Modality::Command => Self::Command,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<Modality, PortError> {
        Ok(match self {
            Self::Text => Modality::Text,
            Self::Image => Modality::Image,
            Self::Code => Modality::Code,
            Self::Pdf => Modality::Pdf,
            Self::Table => Modality::Table,
            Self::Web => Modality::Web,
            Self::Command => Modality::Command,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredModalitySet {
    pub(crate) values: Vec<StoredModality>,
}

impl StoredModalitySet {
    pub(crate) fn from_domain(value: &ModalitySet) -> Self {
        Self {
            values: value
                .values()
                .iter()
                .map(StoredModality::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<ModalitySet, PortError> {
        let values = self
            .values
            .into_iter()
            .map(StoredModality::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ModalitySet::new(values))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchStage {
    InitialRetrieval,
    Reranking,
    Filtering,
    Synthesis,
}

impl StoredSearchStage {
    pub(crate) fn from_domain(value: &SearchStage) -> Self {
        match value {
            SearchStage::InitialRetrieval => Self::InitialRetrieval,
            SearchStage::Reranking => Self::Reranking,
            SearchStage::Filtering => Self::Filtering,
            SearchStage::Synthesis => Self::Synthesis,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchStage, PortError> {
        Ok(match self {
            Self::InitialRetrieval => SearchStage::InitialRetrieval,
            Self::Reranking => SearchStage::Reranking,
            Self::Filtering => SearchStage::Filtering,
            Self::Synthesis => SearchStage::Synthesis,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchPlan {
    pub(crate) query_id: u64,
    pub(crate) original_query: String,
    pub(crate) intent: StoredSearchIntent,
    pub(crate) scope: StoredCorpusScope,
    pub(crate) corpus_snapshot: u64,
    pub(crate) index_generation: u64,
    pub(crate) freshness: StoredFreshnessRequirement,
    pub(crate) modalities: StoredModalitySet,
    pub(crate) stages: Vec<StoredSearchStage>,
    pub(crate) budgets: StoredSearchBudget,
    pub(crate) stop_conditions: StoredStopConditions,
    pub(crate) evidence_requirements: StoredEvidenceRequirements,
    pub(crate) fingerprint: StoredRetrievalModelFingerprint,
    /// Trusted request-bound authorization captured when the plan was created.
    pub(crate) authorization: Option<StoredRetrievalPolicySnapshot>,
    pub(crate) original_intent: Option<StoredSearchIntent>,
    pub(crate) route_decision: Option<StoredSearchRouteDecision>,
}

impl StoredSearchPlan {
    pub(crate) fn from_domain(value: &SearchPlan) -> Self {
        Self {
            query_id: value.query_id().value(),
            original_query: value.original_query().to_string(),
            intent: StoredSearchIntent::from_domain(&value.intent()),
            scope: StoredCorpusScope::from_domain(value.scope()),
            corpus_snapshot: value.corpus_snapshot().value(),
            index_generation: value.index_generation().value(),
            freshness: StoredFreshnessRequirement::from_domain(value.freshness()),
            modalities: StoredModalitySet::from_domain(value.modalities()),
            stages: value
                .stages()
                .iter()
                .map(StoredSearchStage::from_domain)
                .collect(),
            budgets: StoredSearchBudget::from_domain(value.budgets()),
            stop_conditions: StoredStopConditions::from_domain(value.stop_conditions()),
            evidence_requirements: StoredEvidenceRequirements::from_domain(
                value.evidence_requirements(),
            ),
            fingerprint: StoredRetrievalModelFingerprint::from_domain(value.fingerprint()),
            authorization: Some(StoredRetrievalPolicySnapshot::from_domain(
                value.authorization(),
            )),
            original_intent: value
                .original_intent()
                .map(|value| StoredSearchIntent::from_domain(&value)),
            route_decision: value
                .route_decision()
                .map(StoredSearchRouteDecision::from_domain),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchPlan, PortError> {
        let authorization = self
            .authorization
            .map(StoredRetrievalPolicySnapshot::try_into_domain)
            .transpose()?
            .ok_or(PortError::InvalidInputContext {
                context: "decode stored search plan",
                source: "authorization snapshot is required".to_string(),
            })?;
        let route_decision = self
            .route_decision
            .map(StoredSearchRouteDecision::try_into_domain)
            .transpose()?;
        SearchPlan::builder()
            .query_id(QueryId::new(self.query_id))
            .original_query(self.original_query)
            .intent(self.intent.try_into_domain()?)
            .scope(self.scope.try_into_domain()?)
            .corpus_snapshot(maestria_domain::CorpusSnapshotId::new(self.corpus_snapshot))
            .index_generation(IndexGenerationId::new(self.index_generation))
            .freshness(self.freshness.try_into_domain()?)
            .modalities(self.modalities.try_into_domain()?)
            .stages(
                self.stages
                    .into_iter()
                    .map(StoredSearchStage::try_into_domain)
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .budgets(self.budgets.try_into_domain()?)
            .stop_conditions(self.stop_conditions.try_into_domain()?)
            .evidence_requirements(self.evidence_requirements.try_into_domain()?)
            .fingerprint(self.fingerprint.try_into_domain()?)
            .authorization(authorization)
            .original_intent(
                self.original_intent
                    .map(StoredSearchIntent::try_into_domain)
                    .transpose()?,
            )
            .route_decision(route_decision)
            .build()
            .map_err(|error| PortError::InvalidInputContext {
                context: "decode stored search plan",
                source: error.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::{
        CorpusScope, EvidenceRequirements, FreshnessRequirement, IndexGenerationId, Modality,
        ModalitySet, QueryId, RetrievalModelFingerprint, RetrievalPolicySnapshot, ScopeId,
        SearchBudget, SearchIntent, SearchPlan, SearchRouteDecision, SearchStage, Sensitivity,
        StopConditions, TrustZone,
    };
    use maestria_ports::PortError;

    use super::*;

    fn sample_plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
        Ok(SearchPlan::builder()
            .query_id(QueryId::new(1))
            .original_query("rust memory model".to_string())
            .intent(SearchIntent::FactualLocal)
            .scope(CorpusScope::Restricted(vec![ScopeId::new(7)]))
            .corpus_snapshot(maestria_domain::CorpusSnapshotId::new(2))
            .index_generation(IndexGenerationId::new(3))
            .freshness(FreshnessRequirement::Realtime)
            .modalities(ModalitySet::new(vec![Modality::Text, Modality::Code]))
            .stages(vec![
                SearchStage::InitialRetrieval,
                SearchStage::Reranking,
                SearchStage::Filtering,
                SearchStage::Synthesis,
            ])
            .budgets(SearchBudget::with_limits(10_000, 5_000, 8, 4, 4)?)
            .stop_conditions(StopConditions {
                max_results: 10,
                min_score_threshold: 600,
            })
            .evidence_requirements(EvidenceRequirements {
                require_primary_sources: true,
                minimum_corroboration: 1,
                required_claims: Vec::new(),
                required_subquestions: Vec::new(),
                minimum_sources: 1,
                minimum_documents: 0,
                minimum_sections: 0,
            })
            .fingerprint(RetrievalModelFingerprint::new("model-v1".to_string())?)
            .authorization(RetrievalPolicySnapshot {
                require_trust_zone: Some(TrustZone::Verified),
                max_sensitivity: Some(Sensitivity::Confidential),
                require_read_allowed: true,
                effective_scopes: Some(vec![ScopeId::new(7)]),
                allow_unscoped_items: false,
            })
            .original_intent(Some(SearchIntent::SemanticDiscovery))
            .route_decision(Some(SearchRouteDecision::LocalTextFallback))
            .build()?)
    }

    #[test]
    fn plan_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let plan = sample_plan()?;
        let stored = StoredSearchPlan::from_domain(&plan);
        assert_eq!(stored.try_into_domain()?, plan);
        Ok(())
    }

    #[test]
    fn plan_without_authorization_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let mut stored = StoredSearchPlan::from_domain(&sample_plan()?);
        stored.authorization = None;
        assert!(matches!(
            stored.try_into_domain(),
            Err(PortError::InvalidInputContext { .. })
        ));
        Ok(())
    }
}
