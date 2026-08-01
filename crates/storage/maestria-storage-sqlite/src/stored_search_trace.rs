//! Stored-wire mirrors of `maestria_domain::search_outcome::trace::*` plus the
//! per-lane `SearchExecution` tree, so the stored event row owns its own wire
//! format instead of embedding maestria_domain types.
//!
//! This module is the facade for the mirror family: it defines the trace
//! record itself (`StoredSearchTrace`, its candidates, expansions, filters and
//! stop reason) and re-exports the sibling stage mirrors registered in
//! `crate::payloads` (`stored_search_trace_lane`, `stored_search_trace_rerank`,
//! `stored_search_trace_diversity`, `stored_search_trace_rewrite`) so every
//! mirror stays importable from `crate::payloads::stored_search_trace`.
//!
//! Nested search types (`StoredSearchIntent`, `StoredCorpusScope`,
//! `StoredEvidenceSpan`, `StoredRetrievalScoreSet`, ...) live in
//! `crate::stored_search` and are reused here. Id newtypes are flattened to
//! their raw `u64`; validated domain construction runs in `try_into_domain`
//! with errors mapped to `maestria_ports::PortError::InvalidInputContext`.

use maestria_domain::{
    ArtifactVersionId, ConflictSetId, CorpusSnapshotId, DuplicateClusterId, EvidenceId,
    IndexGenerationId, QueryId, SearchStopReason, SearchTrace, SearchTraceCandidate,
    SearchTraceExpansion, SearchTraceFilter,
};
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search::{
    StoredCorpusScope, StoredEvidenceRequirements, StoredEvidenceSpan, StoredFreshnessRequirement,
    StoredFreshnessStatus, StoredModalitySet, StoredRetrievalModelFingerprint,
    StoredRetrievalReason, StoredRetrievalScoreSet, StoredSearchBudget, StoredSearchIntent,
    StoredSearchStage, StoredStopConditions, StoredTrustLabel,
};

pub(crate) use crate::payloads::stored_search_trace_diversity::*;
pub(crate) use crate::payloads::stored_search_trace_lane::*;
pub(crate) use crate::payloads::stored_search_trace_rerank::*;
pub(crate) use crate::payloads::stored_search_trace_rewrite::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchTraceFilter {
    Scope,
    Acl,
    Trust,
    Sensitivity,
    Quarantine,
    PromptInjection,
    Freshness,
}

impl StoredSearchTraceFilter {
    pub(crate) fn from_domain(value: &SearchTraceFilter) -> Self {
        match value {
            SearchTraceFilter::Scope => Self::Scope,
            SearchTraceFilter::Acl => Self::Acl,
            SearchTraceFilter::Trust => Self::Trust,
            SearchTraceFilter::Sensitivity => Self::Sensitivity,
            SearchTraceFilter::Quarantine => Self::Quarantine,
            SearchTraceFilter::PromptInjection => Self::PromptInjection,
            SearchTraceFilter::Freshness => Self::Freshness,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceFilter, maestria_ports::PortError> {
        Ok(match self {
            Self::Scope => SearchTraceFilter::Scope,
            Self::Acl => SearchTraceFilter::Acl,
            Self::Trust => SearchTraceFilter::Trust,
            Self::Sensitivity => SearchTraceFilter::Sensitivity,
            Self::Quarantine => SearchTraceFilter::Quarantine,
            Self::PromptInjection => SearchTraceFilter::PromptInjection,
            Self::Freshness => SearchTraceFilter::Freshness,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceCandidate {
    evidence_id: u64,
    artifact_version: u64,
    source_span: StoredEvidenceSpan,
    rank: u32,
    scores: StoredRetrievalScoreSet,
    trust: StoredTrustLabel,
    freshness: StoredFreshnessStatus,
    duplicate_cluster: Option<u64>,
    reasons: Vec<StoredRetrievalReason>,
    coverage_keys: Vec<String>,
}

impl StoredSearchTraceCandidate {
    pub(crate) fn from_domain(value: &SearchTraceCandidate) -> Self {
        Self {
            evidence_id: value.evidence_id.value(),
            artifact_version: value.artifact_version.value(),
            source_span: StoredEvidenceSpan::from_domain(&value.source_span),
            rank: value.rank,
            scores: StoredRetrievalScoreSet::from_domain(&value.scores),
            trust: StoredTrustLabel::from_domain(&value.trust),
            freshness: StoredFreshnessStatus::from_domain(&value.freshness),
            duplicate_cluster: value.duplicate_cluster.map(|id| id.value()),
            reasons: value
                .reasons
                .iter()
                .map(StoredRetrievalReason::from_domain)
                .collect(),
            coverage_keys: value.coverage_keys.clone(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceCandidate, maestria_ports::PortError> {
        Ok(SearchTraceCandidate {
            evidence_id: EvidenceId::new(self.evidence_id),
            artifact_version: ArtifactVersionId::new(self.artifact_version),
            source_span: self.source_span.try_into_domain()?,
            rank: self.rank,
            scores: self.scores.try_into_domain()?,
            trust: self.trust.try_into_domain()?,
            freshness: self.freshness.try_into_domain()?,
            duplicate_cluster: self.duplicate_cluster.map(DuplicateClusterId::new),
            reasons: self
                .reasons
                .into_iter()
                .map(StoredRetrievalReason::try_into_domain)
                .collect::<Result<_, _>>()?,
            coverage_keys: self.coverage_keys,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceExpansion {
    strategy: String,
    added_candidates: Option<u32>,
}

impl StoredSearchTraceExpansion {
    pub(crate) fn from_domain(value: &SearchTraceExpansion) -> Self {
        Self {
            strategy: value.strategy.clone(),
            added_candidates: value.added_candidates,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceExpansion, maestria_ports::PortError> {
        Ok(SearchTraceExpansion {
            strategy: self.strategy,
            added_candidates: self.added_candidates,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchStopReason {
    ResultsLimit,
    EvidenceComplete,
    RequirementsUnmet,
    NoEvidence,
    LowMarginalGain,
    BudgetExhausted,
    PolicyDenied,
    Abstained,
}

impl StoredSearchStopReason {
    pub(crate) fn from_domain(value: &SearchStopReason) -> Self {
        match value {
            SearchStopReason::ResultsLimit => Self::ResultsLimit,
            SearchStopReason::EvidenceComplete => Self::EvidenceComplete,
            SearchStopReason::RequirementsUnmet => Self::RequirementsUnmet,
            SearchStopReason::NoEvidence => Self::NoEvidence,
            SearchStopReason::LowMarginalGain => Self::LowMarginalGain,
            SearchStopReason::BudgetExhausted => Self::BudgetExhausted,
            SearchStopReason::PolicyDenied => Self::PolicyDenied,
            SearchStopReason::Abstained => Self::Abstained,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchStopReason, maestria_ports::PortError> {
        Ok(match self {
            Self::ResultsLimit => SearchStopReason::ResultsLimit,
            Self::EvidenceComplete => SearchStopReason::EvidenceComplete,
            Self::RequirementsUnmet => SearchStopReason::RequirementsUnmet,
            Self::NoEvidence => SearchStopReason::NoEvidence,
            Self::LowMarginalGain => SearchStopReason::LowMarginalGain,
            Self::BudgetExhausted => SearchStopReason::BudgetExhausted,
            Self::PolicyDenied => SearchStopReason::PolicyDenied,
            Self::Abstained => SearchStopReason::Abstained,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTrace {
    query_id: u64,
    original_query: String,
    intent: StoredSearchIntent,
    original_intent: Option<StoredSearchIntent>,
    unavailable_capability: Option<String>,
    route_decision: Option<String>,
    scope: StoredCorpusScope,
    corpus_snapshot: u64,
    index_generation: u64,
    freshness: StoredFreshnessRequirement,
    modalities: StoredModalitySet,
    degradation: Option<String>,
    stages: Vec<StoredSearchStage>,
    budgets: StoredSearchBudget,
    stop_conditions: StoredStopConditions,
    evidence_requirements: StoredEvidenceRequirements,
    fingerprint: StoredRetrievalModelFingerprint,
    identity_version: u16,
    retrievers: Vec<String>,
    policy_fingerprint: Option<String>,
    raw_candidates: Vec<StoredSearchTraceCandidate>,
    fusion: Option<String>,
    filters: Vec<StoredSearchTraceFilter>,
    expansions: Vec<StoredSearchTraceExpansion>,
    rewrites: Vec<StoredSearchTraceRewrite>,
    missing_evidence: Vec<String>,
    conflicts: Vec<u64>,
    stop_reason: StoredSearchStopReason,
    lanes: Vec<StoredSearchTraceLane>,
    rerank: Option<StoredSearchTraceRerank>,
    diversity: Option<StoredSearchTraceDiversity>,
}

impl StoredSearchTrace {
    pub(crate) fn from_domain(value: &SearchTrace) -> Self {
        Self {
            query_id: value.query_id.value(),
            original_query: value.original_query.clone(),
            intent: StoredSearchIntent::from_domain(&value.intent),
            original_intent: value
                .original_intent
                .as_ref()
                .map(StoredSearchIntent::from_domain),
            unavailable_capability: value.unavailable_capability.clone(),
            route_decision: value.route_decision.clone(),
            scope: StoredCorpusScope::from_domain(&value.scope),
            corpus_snapshot: value.corpus_snapshot.value(),
            index_generation: value.index_generation.value(),
            freshness: StoredFreshnessRequirement::from_domain(&value.freshness),
            modalities: StoredModalitySet::from_domain(&value.modalities),
            degradation: value.degradation.clone(),
            stages: value
                .stages
                .iter()
                .map(StoredSearchStage::from_domain)
                .collect(),
            budgets: StoredSearchBudget::from_domain(&value.budgets),
            stop_conditions: StoredStopConditions::from_domain(&value.stop_conditions),
            evidence_requirements: StoredEvidenceRequirements::from_domain(
                &value.evidence_requirements,
            ),
            fingerprint: StoredRetrievalModelFingerprint::from_domain(&value.fingerprint),
            identity_version: value.identity_version,
            retrievers: value.retrievers.clone(),
            policy_fingerprint: value.policy_fingerprint.clone(),
            raw_candidates: value
                .raw_candidates
                .iter()
                .map(StoredSearchTraceCandidate::from_domain)
                .collect(),
            fusion: value.fusion.clone(),
            filters: value
                .filters
                .iter()
                .map(StoredSearchTraceFilter::from_domain)
                .collect(),
            expansions: value
                .expansions
                .iter()
                .map(StoredSearchTraceExpansion::from_domain)
                .collect(),
            rewrites: value
                .rewrites
                .iter()
                .map(StoredSearchTraceRewrite::from_domain)
                .collect(),
            missing_evidence: value.missing_evidence.clone(),
            conflicts: value.conflicts.iter().map(|id| id.value()).collect(),
            stop_reason: StoredSearchStopReason::from_domain(&value.stop_reason),
            lanes: value
                .lanes
                .iter()
                .map(StoredSearchTraceLane::from_domain)
                .collect(),
            rerank: value
                .rerank
                .as_ref()
                .map(StoredSearchTraceRerank::from_domain),
            diversity: value
                .diversity
                .as_ref()
                .map(StoredSearchTraceDiversity::from_domain),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTrace, maestria_ports::PortError> {
        Ok(SearchTrace {
            query_id: QueryId::new(self.query_id),
            original_query: self.original_query,
            intent: self.intent.try_into_domain()?,
            original_intent: self
                .original_intent
                .map(StoredSearchIntent::try_into_domain)
                .transpose()?,
            unavailable_capability: self.unavailable_capability,
            route_decision: self.route_decision,
            scope: self.scope.try_into_domain()?,
            corpus_snapshot: CorpusSnapshotId::new(self.corpus_snapshot),
            index_generation: IndexGenerationId::new(self.index_generation),
            freshness: self.freshness.try_into_domain()?,
            modalities: self.modalities.try_into_domain()?,
            degradation: self.degradation,
            stages: self
                .stages
                .into_iter()
                .map(StoredSearchStage::try_into_domain)
                .collect::<Result<_, _>>()?,
            budgets: self.budgets.try_into_domain()?,
            stop_conditions: self.stop_conditions.try_into_domain()?,
            evidence_requirements: self.evidence_requirements.try_into_domain()?,
            fingerprint: self.fingerprint.try_into_domain()?,
            identity_version: self.identity_version,
            retrievers: self.retrievers,
            policy_fingerprint: self.policy_fingerprint,
            raw_candidates: self
                .raw_candidates
                .into_iter()
                .map(StoredSearchTraceCandidate::try_into_domain)
                .collect::<Result<_, _>>()?,
            fusion: self.fusion,
            filters: self
                .filters
                .into_iter()
                .map(StoredSearchTraceFilter::try_into_domain)
                .collect::<Result<_, _>>()?,
            expansions: self
                .expansions
                .into_iter()
                .map(StoredSearchTraceExpansion::try_into_domain)
                .collect::<Result<_, _>>()?,
            rewrites: self
                .rewrites
                .into_iter()
                .map(StoredSearchTraceRewrite::try_into_domain)
                .collect::<Result<_, _>>()?,
            missing_evidence: self.missing_evidence,
            conflicts: self.conflicts.into_iter().map(ConflictSetId::new).collect(),
            stop_reason: self.stop_reason.try_into_domain()?,
            lanes: self
                .lanes
                .into_iter()
                .map(StoredSearchTraceLane::try_into_domain)
                .collect::<Result<_, _>>()?,
            rerank: self
                .rerank
                .map(StoredSearchTraceRerank::try_into_domain)
                .transpose()?,
            diversity: self
                .diversity
                .map(StoredSearchTraceDiversity::try_into_domain)
                .transpose()?,
        })
    }
}

#[cfg(test)]
pub(crate) mod stored_search_trace_tests;

#[cfg(test)]
mod tests {
    use maestria_domain::{SearchStopReason, SearchTraceFilter};

    use super::stored_search_trace_tests::sample_trace;
    use super::*;

    #[test]
    fn search_trace_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = sample_trace()?;
        let stored = StoredSearchTrace::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn stored_search_trace_json_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let stored = StoredSearchTrace::from_domain(&sample_trace()?);
        let json = serde_json::to_string(&stored)?;
        let decoded: StoredSearchTrace = serde_json::from_str(&json)?;
        assert_eq!(decoded, stored);
        Ok(())
    }

    #[test]
    fn every_enum_variant_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        for filter in [
            SearchTraceFilter::Scope,
            SearchTraceFilter::Acl,
            SearchTraceFilter::Trust,
            SearchTraceFilter::Sensitivity,
            SearchTraceFilter::Quarantine,
            SearchTraceFilter::PromptInjection,
            SearchTraceFilter::Freshness,
        ] {
            assert_eq!(
                StoredSearchTraceFilter::from_domain(&filter).try_into_domain()?,
                filter
            );
        }
        for reason in [
            SearchStopReason::ResultsLimit,
            SearchStopReason::EvidenceComplete,
            SearchStopReason::RequirementsUnmet,
            SearchStopReason::NoEvidence,
            SearchStopReason::LowMarginalGain,
            SearchStopReason::BudgetExhausted,
            SearchStopReason::PolicyDenied,
            SearchStopReason::Abstained,
        ] {
            assert_eq!(
                StoredSearchStopReason::from_domain(&reason).try_into_domain()?,
                reason
            );
        }
        Ok(())
    }
}
