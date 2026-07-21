use serde::{Deserialize, Serialize};

use super::{RetrievalModelFingerprint, RetrievalScoreSet, SearchCompatibilityError, SearchPlan};
use crate::ids::{
    ArtifactVersionId, ConflictSetId, DuplicateClusterId, EvidenceId, IndexGenerationId,
    SearchTraceId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseContribution {
    pub term_id: u32,
    pub contribution_micros: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LearnedSparseReason {
    pub contributions: Vec<LearnedSparseContribution>,
    #[serde(skip_serializing)]
    legacy_score: Option<LegacyLearnedSparseScore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyLearnedSparseScore {
    score_micros: u32,
    representation: crate::generations::RepresentationName,
    fingerprint: RetrievalModelFingerprint,
}

impl LearnedSparseReason {
    pub fn new(contributions: Vec<LearnedSparseContribution>) -> Self {
        Self {
            contributions,
            legacy_score: None,
        }
    }

    fn take_legacy_score(&mut self) -> Option<LegacyLearnedSparseScore> {
        self.legacy_score.take()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentLearnedSparseReasonDto {
    contributions: Vec<LearnedSparseContribution>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyLearnedSparseReasonDto {
    score_micros: u32,
    representation: crate::generations::RepresentationName,
    fingerprint: RetrievalModelFingerprint,
    contributions: Vec<LearnedSparseContribution>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LearnedSparseReasonWire {
    Current(CurrentLearnedSparseReasonDto),
    Legacy(LegacyLearnedSparseReasonDto),
}

impl<'de> Deserialize<'de> for LearnedSparseReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match LearnedSparseReasonWire::deserialize(deserializer)? {
            LearnedSparseReasonWire::Current(dto) => Self::new(dto.contributions),
            LearnedSparseReasonWire::Legacy(dto) => Self {
                contributions: dto.contributions,
                legacy_score: Some(LegacyLearnedSparseScore {
                    score_micros: dto.score_micros,
                    representation: dto.representation,
                    fingerprint: dto.fingerprint,
                }),
            },
        })
    }
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
    LexicalMatch,
    SemanticSimilarity,
    CitationLink,
    GraphTraversal,
    LateInteraction,
    SpecializedRetrieval { route: String },
    LearnedSparse(Box<LearnedSparseReason>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceCandidateDto")]
pub struct EvidenceCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: super::EvidenceSpan,
    pub scores: RetrievalScoreSet,
    pub trust: TrustLabel,
    pub freshness: FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<RetrievalReason>,
    #[serde(default)]
    pub coverage_keys: Vec<String>,
}

#[derive(Deserialize)]
struct EvidenceCandidateDto {
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: super::EvidenceSpan,
    scores: RetrievalScoreSet,
    trust: TrustLabel,
    freshness: FreshnessStatus,
    duplicate_cluster: Option<DuplicateClusterId>,
    reasons: Vec<RetrievalReason>,
    #[serde(default)]
    coverage_keys: Vec<String>,
}

impl TryFrom<EvidenceCandidateDto> for EvidenceCandidate {
    type Error = SearchCompatibilityError;

    fn try_from(dto: EvidenceCandidateDto) -> Result<Self, Self::Error> {
        let mut candidate = Self {
            evidence_id: dto.evidence_id,
            artifact_version: dto.artifact_version,
            source_span: dto.source_span,
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

impl EvidenceCandidate {
    pub fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        canonicalize_candidate_scores(&mut self.scores, &mut self.reasons)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceCoverageDto")]
pub struct EvidenceCoverage {
    pub percent_covered: u8,
    pub gaps_identified: Vec<String>,
    #[serde(default)]
    pub required_claims: Vec<String>,
    #[serde(default)]
    pub required_subquestions: Vec<String>,
    #[serde(default)]
    pub distinct_sources: usize,
    #[serde(default)]
    pub distinct_documents: usize,
    #[serde(default)]
    pub distinct_sections: usize,
    #[serde(default)]
    pub candidate_coverage_keys: Vec<String>,
}

#[derive(Deserialize)]
struct EvidenceCoverageDto {
    percent_covered: u8,
    gaps_identified: Vec<String>,
    #[serde(default)]
    required_claims: Vec<String>,
    #[serde(default)]
    required_subquestions: Vec<String>,
    #[serde(default)]
    distinct_sources: usize,
    #[serde(default)]
    distinct_documents: usize,
    #[serde(default)]
    distinct_sections: usize,
    #[serde(default)]
    candidate_coverage_keys: Vec<String>,
}

impl TryFrom<EvidenceCoverageDto> for EvidenceCoverage {
    type Error = SearchCompatibilityError;
    fn try_from(dto: EvidenceCoverageDto) -> Result<Self, Self::Error> {
        if dto.percent_covered > 100 {
            return Err(SearchCompatibilityError::InvalidCoverage(
                "percent_covered must be between 0 and 100",
            ));
        }
        Ok(EvidenceCoverage {
            percent_covered: dto.percent_covered,
            gaps_identified: dto.gaps_identified,
            required_claims: dto.required_claims,
            required_subquestions: dto.required_subquestions,
            distinct_sources: dto.distinct_sources,
            distinct_documents: dto.distinct_documents,
            distinct_sections: dto.distinct_sections,
            candidate_coverage_keys: dto.candidate_coverage_keys,
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
#[serde(try_from = "SearchTraceCandidateDto")]
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
    #[serde(default)]
    pub coverage_keys: Vec<String>,
}

#[derive(Deserialize)]
struct SearchTraceCandidateDto {
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: super::EvidenceSpan,
    rank: u32,
    scores: RetrievalScoreSet,
    trust: TrustLabel,
    freshness: FreshnessStatus,
    duplicate_cluster: Option<DuplicateClusterId>,
    reasons: Vec<RetrievalReason>,
    #[serde(default)]
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
        canonicalize_candidate_scores(&mut self.scores, &mut self.reasons)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceExpansion {
    pub strategy: String,
    pub added_candidates: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchRewriteOrigin {
    Original,
    Deterministic,
    ModelProposal,
    Feedback,
    MissingSlot,
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
    #[serde(default)]
    pub missing_slot: Option<String>,
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
    pub source_span: super::EvidenceSpan,
    pub lane_rank: u32,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub scores: RetrievalScoreSet,
    pub reasons: Vec<RetrievalReason>,
}

#[derive(Deserialize)]
struct SearchTraceLaneCandidateDto {
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: super::EvidenceSpan,
    lane_rank: u32,
    duplicate_cluster: Option<DuplicateClusterId>,
    scores: RetrievalScoreSet,
    reasons: Vec<RetrievalReason>,
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
        canonicalize_candidate_scores(&mut self.scores, &mut self.reasons)
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
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub generation: Option<crate::ids::IndexGenerationId>,
    pub status: SearchLaneStatus,
    pub candidates: Vec<SearchTraceLaneCandidate>,
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
    #[serde(default)]
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
    #[serde(default)]
    pub coverage_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceDiversity {
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
    pub required_claims: Vec<String>,
    pub required_subquestions: Vec<String>,
    #[serde(default)]
    pub covered_keys: Vec<String>,
    pub stop_reason: SearchStopReason,
    pub candidates: Vec<SearchTraceDiversityCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub query_id: crate::ids::QueryId,
    pub original_query: String,
    pub intent: super::SearchIntent,
    #[serde(default)]
    pub original_intent: Option<super::SearchIntent>,
    #[serde(default)]
    pub unavailable_capability: Option<String>,
    #[serde(default)]
    pub route_decision: Option<String>,
    pub scope: super::CorpusScope,
    pub corpus_snapshot: crate::ids::CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub freshness: super::FreshnessRequirement,
    pub modalities: super::ModalitySet,
    /// Explicit capability degradation, such as visual retrieval falling back
    /// to text/layout retrieval when no visual provider is available.
    #[serde(default)]
    pub degradation: Option<String>,
    pub stages: Vec<super::SearchStage>,
    pub budgets: super::SearchBudget,
    pub stop_conditions: super::StopConditions,
    pub evidence_requirements: super::EvidenceRequirements,
    pub fingerprint: RetrievalModelFingerprint,
    #[serde(default)]
    pub identity_version: u16,
    pub retrievers: Vec<String>,
    pub policy_fingerprint: Option<String>,
    pub raw_candidates: Vec<SearchTraceCandidate>,
    pub fusion: Option<String>,
    pub filters: Vec<SearchTraceFilter>,
    pub expansions: Vec<SearchTraceExpansion>,
    #[serde(default)]
    pub rewrites: Vec<SearchTraceRewrite>,
    pub missing_evidence: Vec<String>,
    pub conflicts: Vec<ConflictSetId>,
    pub stop_reason: SearchStopReason,
    #[serde(default)]
    pub lanes: Vec<SearchTraceLane>,
    #[serde(default)]
    pub rerank: Option<SearchTraceRerank>,
    #[serde(default)]
    pub diversity: Option<SearchTraceDiversity>,
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
            intent: plan.intent,
            original_intent: plan.original_intent,
            unavailable_capability: None,
            route_decision: plan.route_decision.clone(),
            scope: plan.scope.clone(),
            corpus_snapshot: plan.corpus_snapshot,
            index_generation: plan.index_generation,
            freshness: plan.freshness.clone(),
            degradation: None,
            modalities: plan.modalities.clone(),
            stages: plan.stages.clone(),
            evidence_requirements: plan.evidence_requirements.clone(),
            fingerprint: plan.fingerprint.clone(),
            identity_version: 6,
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
                    coverage_keys: candidate.coverage_keys.clone(),
                })
                .collect(),
            fusion,
            rewrites: vec![SearchTraceRewrite {
                query: plan.original_query.clone(),
                origin: SearchRewriteOrigin::Original,
                stage: SearchRewriteStage::InitialRetrieval,
                accounting: SearchRewriteAccounting {
                    token_estimate: plan
                        .original_query
                        .split_whitespace()
                        .count()
                        .max(1)
                        .min(u32::MAX as usize) as u32,
                    latency_budget_units: 1,
                    is_proposal: false,
                },
                missing_slot: None,
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
        self.identity_version = self.identity_version.max(6);
        self.degradation = Some(degradation.into());
        self
    }
    pub fn with_unavailable_capability(mut self, capability: impl Into<String>) -> Self {
        self.identity_version = self.identity_version.max(6);
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
        self.identity_version = 6;
        Ok(())
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

impl SearchOutcome {
    pub fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        for candidate in &mut self.evidence {
            candidate.canonicalize_score_provenance()?;
        }
        for conflict in &mut self.conflicts {
            for candidate in &mut conflict.candidates {
                candidate.canonicalize_score_provenance()?;
            }
        }
        if let Some(trace) = &mut self.trace_data {
            trace.canonicalize_score_provenance()?;
            self.trace = trace.deterministic_id();
        }
        Ok(())
    }
}

fn canonicalize_candidate_scores(
    scores: &mut RetrievalScoreSet,
    reasons: &mut [RetrievalReason],
) -> Result<(), SearchCompatibilityError> {
    scores.canonicalize()?;
    for reason in reasons {
        let RetrievalReason::LearnedSparse(reason) = reason else {
            continue;
        };
        let Some(legacy) = reason.take_legacy_score() else {
            continue;
        };
        if scores
            .lane(&super::RetrievalScoreKind::LearnedSparse)
            .is_some()
        {
            continue;
        }
        let representation = legacy.representation;
        scores.upsert(super::RetrievalLaneScore::new(
            super::RetrievalScoreKind::LearnedSparse,
            i64::from(legacy.score_micros),
            super::RetrievalRawRank::unavailable(
                "legacy learned-sparse reason did not retain the backend rank",
            ),
            super::RetrievalScoreScale::unbounded("legacy_sparse_score_micros"),
            representation.clone(),
            super::RetrievalScoreFingerprint::new(
                legacy.fingerprint,
                std::collections::BTreeMap::from([
                    ("migration".to_string(), "score_schema_v1_to_v2".to_string()),
                    ("representation".to_string(), representation.0),
                ]),
            ),
        ))?;
    }
    Ok(())
}
