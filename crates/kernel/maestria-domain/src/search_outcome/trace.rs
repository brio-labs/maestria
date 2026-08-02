use serde::{Deserialize, Serialize};

use crate::ids::{
    ArtifactVersionId, ConflictSetId, DuplicateClusterId, EvidenceId, IndexGenerationId,
};
use crate::search::search_outcome::candidate::canonicalize_candidate_scores;
use crate::search::{
    CorpusScope, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, ModalitySet,
    RetrievalModelFingerprint, RetrievalScoreSet, SearchBudget, SearchCompatibilityError,
    SearchPlan, SearchStage, StopConditions,
};

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
#[serde(try_from = "SearchTraceCandidateDto")]
pub struct SearchTraceCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub rank: u32,
    pub scores: RetrievalScoreSet,
    pub trust: super::TrustLabel,
    pub freshness: super::FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<super::RetrievalReason>,
    pub coverage_keys: Vec<String>,
}

#[derive(Deserialize)]
struct SearchTraceCandidateDto {
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: EvidenceSpan,
    rank: u32,
    scores: RetrievalScoreSet,
    trust: super::TrustLabel,
    freshness: super::FreshnessStatus,
    duplicate_cluster: Option<DuplicateClusterId>,
    reasons: Vec<super::RetrievalReason>,
    coverage_keys: Vec<String>,
}

impl TryFrom<SearchTraceCandidateDto> for SearchTraceCandidate {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchTraceCandidateDto) -> Result<Self, Self::Error> {
        let mut candidate = Self {
            evidence_id: dto.evidence_id,
            artifact_version: dto.artifact_version,
            source_span: dto.source_span,
            rank: dto.rank,
            scores: dto.scores,
            trust: dto.trust,
            freshness: dto.freshness,
            duplicate_cluster: dto.duplicate_cluster,
            reasons: dto.reasons,
            coverage_keys: dto.coverage_keys,
        };
        candidate.canonicalize_score_provenance()?;
        Ok(candidate)
    }
}

impl SearchTraceCandidate {
    fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        canonicalize_candidate_scores(&mut self.scores)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceExpansion {
    pub strategy: String,
    pub added_candidates: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchRewriteOrigin {
    Original,
    Deterministic,
    ModelProposal,
    Feedback,
    /// A rewrite that fills a declared missing-evidence slot; the slot
    /// identity lives on the variant so a missing-slot rewrite without a
    /// named slot is unrepresentable (R56).
    MissingSlot {
        slot: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchRewriteStage {
    InitialRetrieval,
    Reranking,
    IterativeRetrieval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRewriteAccounting {
    pub token_estimate: u32,
    pub latency_budget_units: u32,
    pub is_proposal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRewrite {
    pub query: String,
    pub origin: SearchRewriteOrigin,
    pub stage: SearchRewriteStage,
    pub accounting: SearchRewriteAccounting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStopReason {
    ResultsLimit,
    EvidenceComplete,
    RequirementsUnmet,
    NoEvidence,
    LowMarginalGain,
    BudgetExhausted,
    PolicyDenied,
    Abstained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchTraceLaneCandidateDto")]
pub struct SearchTraceLaneCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub lane_rank: u32,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub scores: RetrievalScoreSet,
    pub reasons: Vec<super::RetrievalReason>,
}

#[derive(Deserialize)]
struct SearchTraceLaneCandidateDto {
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: EvidenceSpan,
    lane_rank: u32,
    duplicate_cluster: Option<DuplicateClusterId>,
    scores: RetrievalScoreSet,
    reasons: Vec<super::RetrievalReason>,
}

impl TryFrom<SearchTraceLaneCandidateDto> for SearchTraceLaneCandidate {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchTraceLaneCandidateDto) -> Result<Self, Self::Error> {
        let mut candidate = Self {
            evidence_id: dto.evidence_id,
            artifact_version: dto.artifact_version,
            source_span: dto.source_span,
            lane_rank: dto.lane_rank,
            duplicate_cluster: dto.duplicate_cluster,
            scores: dto.scores,
            reasons: dto.reasons,
        };
        candidate.canonicalize_score_provenance()?;
        Ok(candidate)
    }
}

impl SearchTraceLaneCandidate {
    fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        canonicalize_candidate_scores(&mut self.scores)
    }
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
    pub query: String,
    pub generation: Option<crate::ids::IndexGenerationId>,
    pub status: SearchLaneStatus,
    pub candidates: Vec<SearchTraceLaneCandidate>,
    pub execution: crate::SearchExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RerankCandidateStatus {
    Reranked,
    SkippedCap,
    SkippedNotApplicable,
    ErrorFallback(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceConstraintScore {
    pub name: String,
    pub score: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRerankCandidate {
    pub candidate_id: crate::ids::EvidenceId,
    pub original_rank: usize,
    pub new_rank: Option<usize>,
    pub status: RerankCandidateStatus,
    pub relevance_score: Option<u32>,
    pub constraint_score: Option<u32>,
    pub constraint_scores: Vec<SearchTraceConstraintScore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRerank {
    pub model: String,
    pub fingerprint: RetrievalModelFingerprint,
    pub input_cap: usize,
    pub score_cap: usize,
    pub output_cap: usize,
    pub candidates: Vec<SearchTraceRerankCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceDiversityCandidate {
    pub candidate_id: crate::ids::EvidenceId,
    pub original_rank: usize,
    pub selected_rank: Option<usize>,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub marginal_coverage: u8,
    pub coverage_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceDiversity {
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
    pub required_claims: Vec<String>,
    pub required_subquestions: Vec<String>,
    pub covered_keys: Vec<String>,
    pub stop_reason: SearchStopReason,
    pub candidates: Vec<SearchTraceDiversityCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub query_id: crate::ids::QueryId,
    pub original_query: String,
    pub intent: crate::search::SearchIntent,
    pub original_intent: Option<crate::search::SearchIntent>,
    pub unavailable_capability: Option<String>,
    pub route_decision: Option<String>,
    pub scope: CorpusScope,
    pub corpus_snapshot: crate::ids::CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub freshness: FreshnessRequirement,
    pub modalities: ModalitySet,
    /// Explicit capability degradation, such as visual retrieval falling back
    /// to text/layout retrieval when no visual provider is available.
    pub degradation: Option<String>,
    pub stages: Vec<SearchStage>,
    pub budgets: SearchBudget,
    pub stop_conditions: StopConditions,
    pub evidence_requirements: EvidenceRequirements,
    pub fingerprint: RetrievalModelFingerprint,
    pub identity_version: u16,
    pub retrievers: Vec<String>,
    pub policy_fingerprint: Option<String>,
    pub raw_candidates: Vec<SearchTraceCandidate>,
    pub fusion: Option<String>,
    pub filters: Vec<SearchTraceFilter>,
    pub expansions: Vec<SearchTraceExpansion>,
    pub rewrites: Vec<SearchTraceRewrite>,
    pub missing_evidence: Vec<String>,
    pub conflicts: Vec<ConflictSetId>,
    pub stop_reason: SearchStopReason,
    pub lanes: Vec<SearchTraceLane>,
    pub rerank: Option<SearchTraceRerank>,
    pub diversity: Option<SearchTraceDiversity>,
}

impl SearchTrace {
    pub fn from_plan(
        plan: &SearchPlan,
        retrievers: Vec<String>,
        evidence: &[super::EvidenceCandidate],
        filters: Vec<SearchTraceFilter>,
        fusion: Option<String>,
        expansions: Vec<SearchTraceExpansion>,
        stop_reason: SearchStopReason,
    ) -> Self {
        Self {
            query_id: plan.query_id(),
            original_query: plan.original_query().to_string(),
            intent: plan.intent(),
            original_intent: plan.original_intent(),
            unavailable_capability: None,
            route_decision: plan.route_decision().map(str::to_string),
            scope: plan.scope().clone(),
            corpus_snapshot: plan.corpus_snapshot(),
            index_generation: plan.index_generation(),
            freshness: plan.freshness().clone(),
            degradation: None,
            modalities: plan.modalities().clone(),
            stages: plan.stages().to_vec(),
            evidence_requirements: plan.evidence_requirements().clone(),
            fingerprint: plan.fingerprint().clone(),
            identity_version: 7,
            retrievers,
            policy_fingerprint: None,
            budgets: plan.budgets().clone(),
            stop_conditions: plan.stop_conditions().clone(),
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
                    coverage_keys: candidate.coverage_keys.clone(),
                })
                .collect(),
            fusion,
            rewrites: vec![SearchTraceRewrite {
                query: plan.original_query().to_string(),
                origin: SearchRewriteOrigin::Original,
                stage: SearchRewriteStage::InitialRetrieval,
                accounting: SearchRewriteAccounting {
                    token_estimate: plan
                        .original_query()
                        .split_whitespace()
                        .count()
                        .max(1)
                        .min(u32::MAX as usize) as u32,
                    latency_budget_units: 1,
                    is_proposal: false,
                },
            }],
            filters,
            expansions,
            missing_evidence: Vec::new(),
            conflicts: Vec::new(),
            stop_reason,
            lanes: Vec::new(),
            rerank: None,
            diversity: None,
        }
    }
    pub fn with_degradation(mut self, degradation: impl Into<String>) -> Self {
        self.identity_version = self.identity_version.max(7);
        self.degradation = Some(degradation.into());
        self
    }
    pub fn with_unavailable_capability(mut self, capability: impl Into<String>) -> Self {
        self.identity_version = self.identity_version.max(7);
        self.unavailable_capability = Some(capability.into());
        self
    }

    pub fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        for candidate in &mut self.raw_candidates {
            candidate.canonicalize_score_provenance()?;
        }
        for lane in &mut self.lanes {
            for candidate in &mut lane.candidates {
                candidate.canonicalize_score_provenance()?;
            }
        }
        self.identity_version = 7;
        Ok(())
    }
}
