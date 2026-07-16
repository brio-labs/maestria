use serde::{Deserialize, Serialize};

use super::{RetrievalModelFingerprint, SearchCompatibilityError, SearchPlan};
use crate::ids::{
    ArtifactVersionId, ConflictSetId, DuplicateClusterId, EvidenceId, IndexGenerationId,
    SearchTraceId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalScoreSet {
    pub bm25: u32,
    pub semantic_similarity: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLabel {
    Verified,
    Unverified,
    Disputed,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreshnessStatus {
    UpToDate,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetrievalReason {
    ExactMatch,
    SemanticSimilarity,
    CitationLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: super::EvidenceSpan,
    pub scores: RetrievalScoreSet,
    pub trust: TrustLabel,
    pub freshness: FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<RetrievalReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceCoverageDto")]
pub struct EvidenceCoverage {
    pub percent_covered: u8,
    pub gaps_identified: Vec<String>,
}

#[derive(Deserialize)]
struct EvidenceCoverageDto {
    percent_covered: u8,
    gaps_identified: Vec<String>,
}

impl TryFrom<EvidenceCoverageDto> for EvidenceCoverage {
    type Error = SearchCompatibilityError;

    fn try_from(dto: EvidenceCoverageDto) -> Result<Self, Self::Error> {
        if dto.percent_covered > 100 {
            return Err(SearchCompatibilityError::InvalidCoverage(
                "percent_covered must be between 0 and 100",
            ));
        }
        Ok(Self {
            percent_covered: dto.percent_covered,
            gaps_identified: dto.gaps_identified,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSet {
    pub id: ConflictSetId,
    pub candidates: Vec<EvidenceCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStatus {
    Answerable,
    AnswerableWithWarnings,
    EvidenceIncomplete,
    SourcesConflict,
    StaleEvidenceOnly,
    NoEvidenceFound,
    Abstained,
    DeniedByPolicy,
    QuarantinedForReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchTraceFilter {
    Scope,
    Acl,
    Trust,
    Sensitivity,
    Quarantine,
    PromptInjection,
    Freshness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: super::EvidenceSpan,
    pub rank: u32,
    pub scores: RetrievalScoreSet,
    pub trust: TrustLabel,
    pub freshness: FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<RetrievalReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceExpansion {
    pub strategy: String,
    pub added_candidates: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStopReason {
    ResultsLimit,
    EvidenceComplete,
    RequirementsUnmet,
    NoEvidence,
    BudgetExhausted,
    PolicyDenied,
    Abstained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceLaneCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: super::EvidenceSpan,
    pub lane_rank: u32,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub scores: RetrievalScoreSet,
    pub reasons: Vec<RetrievalReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchLaneStatus {
    Succeeded,
    Empty,
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceLane {
    pub retriever_id: String,
    pub status: SearchLaneStatus,
    pub candidates: Vec<SearchTraceLaneCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub query_id: crate::ids::QueryId,
    pub original_query: String,
    pub intent: super::SearchIntent,
    pub scope: super::CorpusScope,
    pub corpus_snapshot: crate::ids::CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub freshness: super::FreshnessRequirement,
    pub modalities: super::ModalitySet,
    pub stages: Vec<super::SearchStage>,
    pub budgets: super::SearchBudget,
    pub stop_conditions: super::StopConditions,
    pub evidence_requirements: super::EvidenceRequirements,
    pub fingerprint: RetrievalModelFingerprint,
    pub retrievers: Vec<String>,
    pub policy_fingerprint: Option<String>,
    pub raw_candidates: Vec<SearchTraceCandidate>,
    pub fusion: Option<String>,
    pub filters: Vec<SearchTraceFilter>,
    pub expansions: Vec<SearchTraceExpansion>,
    pub missing_evidence: Vec<String>,
    pub conflicts: Vec<ConflictSetId>,
    pub stop_reason: SearchStopReason,
    #[serde(default)]
    pub lanes: Vec<SearchTraceLane>,
}

impl SearchTrace {
    pub fn from_plan(
        plan: &SearchPlan,
        retrievers: Vec<String>,
        evidence: &[EvidenceCandidate],
        filters: Vec<SearchTraceFilter>,
        fusion: Option<String>,
        expansions: Vec<SearchTraceExpansion>,
        stop_reason: SearchStopReason,
    ) -> Self {
        Self {
            query_id: plan.query_id,
            original_query: plan.original_query.clone(),
            intent: plan.intent.clone(),
            scope: plan.scope.clone(),
            corpus_snapshot: plan.corpus_snapshot,
            index_generation: plan.index_generation,
            freshness: plan.freshness.clone(),
            modalities: plan.modalities.clone(),
            stages: plan.stages.clone(),
            evidence_requirements: plan.evidence_requirements.clone(),
            fingerprint: plan.fingerprint.clone(),
            retrievers,
            policy_fingerprint: None,
            budgets: plan.budgets.clone(),
            stop_conditions: plan.stop_conditions.clone(),
            raw_candidates: evidence
                .iter()
                .enumerate()
                .map(|(rank, candidate)| SearchTraceCandidate {
                    evidence_id: candidate.evidence_id,
                    artifact_version: candidate.artifact_version,
                    source_span: candidate.source_span.clone(),
                    rank: rank as u32,
                    scores: candidate.scores.clone(),
                    trust: candidate.trust.clone(),
                    freshness: candidate.freshness.clone(),
                    duplicate_cluster: candidate.duplicate_cluster,
                    reasons: candidate.reasons.clone(),
                })
                .collect(),
            fusion,
            filters,
            expansions,
            missing_evidence: Vec::new(),
            conflicts: Vec::new(),
            stop_reason,
            lanes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub trace: SearchTraceId,
    #[serde(default)]
    pub trace_data: Option<Box<SearchTrace>>,
    pub fingerprint: RetrievalModelFingerprint,
    pub index_generation: IndexGenerationId,
    pub status: SearchStatus,
    pub evidence: Vec<EvidenceCandidate>,
    pub coverage: EvidenceCoverage,
    pub conflicts: Vec<ConflictSet>,
}
