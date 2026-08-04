use serde::{Deserialize, Serialize};

use crate::ids::{
    ArtifactVersionId, ConflictSetId, DuplicateClusterId, EvidenceId, IndexGenerationId,
};
use crate::search::search_outcome::candidate::canonicalize_candidate_scores;
use crate::search::search_outcome::expansion::SearchTraceExpansion;
use crate::search::search_outcome::rerank::SearchTraceRerank;
use crate::search::search_outcome::rewrite::{
    SearchRewriteAccounting, SearchRewriteOrigin, SearchRewriteStage, SearchStopReason,
    SearchTraceFilter, SearchTraceRewrite,
};
use crate::search::{
    CorpusScope, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, ModalitySet,
    RetrievalModelFingerprint, RetrievalScoreSet, SearchBudget, SearchCompatibilityError,
    SearchPlan, SearchStage, StopConditions,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchTraceCandidateDto")]
pub struct SearchTraceCandidate {
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

impl SearchTraceCandidate {
    /// Validate and construct a trace candidate from its boundary input;
    /// score provenance is canonicalized before the value exists (R56:
    /// fields are private).
    pub fn new(dto: SearchTraceCandidateDto) -> Result<Self, SearchCompatibilityError> {
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

    pub fn evidence_id(&self) -> EvidenceId {
        self.evidence_id
    }

    pub fn artifact_version(&self) -> ArtifactVersionId {
        self.artifact_version
    }

    pub fn source_span(&self) -> &EvidenceSpan {
        &self.source_span
    }

    pub fn rank(&self) -> u32 {
        self.rank
    }

    pub fn scores(&self) -> &RetrievalScoreSet {
        &self.scores
    }

    pub fn trust(&self) -> super::TrustLabel {
        self.trust.clone()
    }

    pub fn freshness(&self) -> super::FreshnessStatus {
        self.freshness.clone()
    }

    pub fn duplicate_cluster(&self) -> Option<DuplicateClusterId> {
        self.duplicate_cluster
    }

    pub fn reasons(&self) -> &[super::RetrievalReason] {
        &self.reasons
    }

    pub fn coverage_keys(&self) -> &[String] {
        &self.coverage_keys
    }

    fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        canonicalize_candidate_scores(&mut self.scores)
    }
}

/// Boundary input for [`SearchTraceCandidate`] (R37).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchTraceCandidateDto {
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

impl TryFrom<SearchTraceCandidateDto> for SearchTraceCandidate {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchTraceCandidateDto) -> Result<Self, Self::Error> {
        Self::new(dto)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchTraceLaneCandidateDto")]
pub struct SearchTraceLaneCandidate {
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: EvidenceSpan,
    lane_rank: u32,
    duplicate_cluster: Option<DuplicateClusterId>,
    scores: RetrievalScoreSet,
    reasons: Vec<super::RetrievalReason>,
}

impl SearchTraceLaneCandidate {
    /// Validate and construct a lane candidate from its boundary input;
    /// score provenance is canonicalized before the value exists (R56:
    /// fields are private).
    pub fn new(dto: SearchTraceLaneCandidateDto) -> Result<Self, SearchCompatibilityError> {
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

    pub fn evidence_id(&self) -> EvidenceId {
        self.evidence_id
    }

    pub fn artifact_version(&self) -> ArtifactVersionId {
        self.artifact_version
    }

    pub fn source_span(&self) -> &EvidenceSpan {
        &self.source_span
    }

    pub fn lane_rank(&self) -> u32 {
        self.lane_rank
    }

    pub fn duplicate_cluster(&self) -> Option<DuplicateClusterId> {
        self.duplicate_cluster
    }

    pub fn scores(&self) -> &RetrievalScoreSet {
        &self.scores
    }

    pub fn reasons(&self) -> &[super::RetrievalReason] {
        &self.reasons
    }

    fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        canonicalize_candidate_scores(&mut self.scores)
    }
}

/// Boundary input for [`SearchTraceLaneCandidate`] (R37).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchTraceLaneCandidateDto {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub lane_rank: u32,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub scores: RetrievalScoreSet,
    pub reasons: Vec<super::RetrievalReason>,
}

impl TryFrom<SearchTraceLaneCandidateDto> for SearchTraceLaneCandidate {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchTraceLaneCandidateDto) -> Result<Self, Self::Error> {
        Self::new(dto)
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

/// Why a diversity candidate was not selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiversitySkipReason {
    /// Candidate belongs to an already-selected duplicate cluster.
    DuplicateCluster,
    /// Requirements are satisfied and the candidate adds no marginal gain.
    LowMarginalGain,
}

/// Final placement of one candidate after diversity selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiversityPlacement {
    /// Candidate selected for the diversified result; carries its selection rank.
    Selected(usize),
    /// Candidate skipped; carries the reason.
    Skipped(DiversitySkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceDiversityCandidate {
    pub candidate_id: crate::ids::EvidenceId,
    pub original_rank: usize,
    pub placement: DiversityPlacement,
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

/// Explicit retrieval degradation: the unavailable capability and the
/// traced fallback reason (Rule 54: unavailable providers degrade with a
/// trace, never an implicit fallback). One value carries both facets, so a
/// degradation can never name a capability without its reason or vice
/// versa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDegradation {
    /// The capability that was unavailable, e.g. `"visual provider"`.
    pub capability: String,
    /// The fallback behavior applied, e.g. text/layout retrieval.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub query_id: crate::ids::QueryId,
    pub original_query: String,
    pub intent: crate::search::SearchIntent,
    pub original_intent: Option<crate::search::SearchIntent>,
    pub route_decision: Option<crate::search::SearchRouteDecision>,
    pub scope: CorpusScope,
    pub corpus_snapshot: crate::ids::CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub freshness: FreshnessRequirement,
    pub modalities: ModalitySet,
    /// Explicit capability degradation, such as visual retrieval falling back
    /// to text/layout retrieval when no visual provider is available.
    pub degradation: Option<SearchDegradation>,
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
    /// Build a trace from a validated plan and its evidence candidates.
    ///
    /// Candidate score provenance is canonicalized during construction and
    /// a typed error surfaces when the evidence set cannot be traced.
    pub fn from_plan(
        plan: &SearchPlan,
        retrievers: Vec<String>,
        evidence: &[super::EvidenceCandidate],
        filters: Vec<SearchTraceFilter>,
        fusion: Option<String>,
        expansions: Vec<SearchTraceExpansion>,
        stop_reason: SearchStopReason,
    ) -> Result<Self, SearchCompatibilityError> {
        let mut trace = Self {
            query_id: plan.query_id(),
            original_query: plan.original_query().to_string(),
            intent: plan.intent(),
            original_intent: plan.original_intent(),
            route_decision: plan.route_decision().cloned(),
            scope: plan.scope().clone(),
            corpus_snapshot: plan.corpus_snapshot(),
            index_generation: plan.index_generation(),
            freshness: plan.freshness().clone(),
            degradation: None,
            modalities: plan.modalities().clone(),
            stages: plan.stages().to_vec(),
            evidence_requirements: plan.evidence_requirements().clone(),
            fingerprint: plan.fingerprint().clone(),
            identity_version: 8,
            retrievers,
            policy_fingerprint: None,
            budgets: plan.budgets().clone(),
            stop_conditions: plan.stop_conditions().clone(),
            raw_candidates: evidence
                .iter()
                .enumerate()
                .map(|(rank, candidate)| {
                    SearchTraceCandidate::new(SearchTraceCandidateDto {
                        evidence_id: candidate.evidence_id(),
                        artifact_version: candidate.artifact_version(),
                        source_span: candidate.source_span().clone(),
                        rank: rank as u32,
                        scores: candidate.scores().clone(),
                        trust: candidate.trust(),
                        freshness: candidate.freshness(),
                        duplicate_cluster: candidate.duplicate_cluster(),
                        reasons: candidate.reasons().to_vec(),
                        coverage_keys: candidate.coverage_keys().to_vec(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
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
        };
        trace.canonicalize_score_provenance()?;
        Ok(trace)
    }
    pub fn with_degradation(mut self, degradation: SearchDegradation) -> Self {
        self.identity_version = self.identity_version.max(8);
        self.degradation = Some(degradation);
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
