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
    Success,
    PartialResults,
    Timeout,
    ExhaustedBudget,
    DeniedByPolicy,
    QuarantinedForReview,
    Abstained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub trace: SearchTraceId,
    pub fingerprint: RetrievalModelFingerprint,
    pub index_generation: IndexGenerationId,
    pub status: SearchStatus,
    pub evidence: Vec<EvidenceCandidate>,
    pub coverage: EvidenceCoverage,
    pub conflicts: Vec<ConflictSet>,
}

impl SearchOutcome {
    pub fn verify_compatibility(&self, plan: &SearchPlan) -> Result<(), SearchCompatibilityError> {
        if self.fingerprint != plan.fingerprint {
            return Err(SearchCompatibilityError::ModelFingerprintMismatch {
                expected: plan.fingerprint.clone(),
                found: self.fingerprint.clone(),
            });
        }
        if self.index_generation != plan.index_generation {
            return Err(SearchCompatibilityError::IndexGenerationMismatch {
                expected: plan.index_generation,
                found: self.index_generation,
            });
        }
        Ok(())
    }
}
