use serde::{Deserialize, Serialize};

use crate::ids::{ArtifactVersionId, DuplicateClusterId, EvidenceId};
use crate::search::{EvidenceSpan, RetrievalScoreSet, SearchCompatibilityError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseContribution {
    pub term_id: u32,
    pub contribution_micros: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearnedSparseReason {
    pub contributions: Vec<LearnedSparseContribution>,
}

impl LearnedSparseReason {
    pub fn new(contributions: Vec<LearnedSparseContribution>) -> Self {
        Self { contributions }
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
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: EvidenceSpan,
    scores: RetrievalScoreSet,
    trust: TrustLabel,
    freshness: FreshnessStatus,
    duplicate_cluster: Option<DuplicateClusterId>,
    reasons: Vec<RetrievalReason>,
    coverage_keys: Vec<String>,
}

impl EvidenceCandidate {
    /// Validate and construct a candidate from its boundary input; score
    /// provenance is canonicalized before the value exists (R56: fields are
    /// private so the conversion cannot be bypassed by a struct literal).
    pub fn new(dto: EvidenceCandidateDto) -> Result<Self, SearchCompatibilityError> {
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

    pub fn evidence_id(&self) -> EvidenceId {
        self.evidence_id
    }

    pub fn artifact_version(&self) -> ArtifactVersionId {
        self.artifact_version
    }

    pub fn source_span(&self) -> &EvidenceSpan {
        &self.source_span
    }

    pub fn scores(&self) -> &RetrievalScoreSet {
        &self.scores
    }

    pub fn trust(&self) -> TrustLabel {
        self.trust.clone()
    }

    pub fn freshness(&self) -> FreshnessStatus {
        self.freshness.clone()
    }

    pub fn duplicate_cluster(&self) -> Option<DuplicateClusterId> {
        self.duplicate_cluster
    }

    pub fn reasons(&self) -> &[RetrievalReason] {
        &self.reasons
    }

    pub fn coverage_keys(&self) -> &[String] {
        &self.coverage_keys
    }
}

/// Boundary input for [`EvidenceCandidate`] (R37): the wire shape is the
/// validated construction input; `TryFrom`/`new` own every invariant.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCandidateDto {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub scores: RetrievalScoreSet,
    pub trust: TrustLabel,
    pub freshness: FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<RetrievalReason>,
    pub coverage_keys: Vec<String>,
}

impl TryFrom<EvidenceCandidateDto> for EvidenceCandidate {
    type Error = SearchCompatibilityError;

    fn try_from(dto: EvidenceCandidateDto) -> Result<Self, Self::Error> {
        Self::new(dto)
    }
}

impl EvidenceCandidate {
    pub fn canonicalize_score_provenance(&mut self) -> Result<(), SearchCompatibilityError> {
        canonicalize_candidate_scores(&mut self.scores)
    }
}

pub(crate) fn canonicalize_candidate_scores(
    scores: &mut RetrievalScoreSet,
) -> Result<(), SearchCompatibilityError> {
    scores.canonicalize()
}
