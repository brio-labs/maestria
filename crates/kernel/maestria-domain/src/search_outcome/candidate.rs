use serde::{Deserialize, Serialize};

use crate::ids::{ArtifactVersionId, DuplicateClusterId, EvidenceId};
use crate::search::{
    EvidenceSpan, RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank,
    RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet,
    SearchCompatibilityError,
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
    pub source_span: EvidenceSpan,
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
    source_span: EvidenceSpan,
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

pub(crate) fn canonicalize_candidate_scores(
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
        if scores.lane(&RetrievalScoreKind::LearnedSparse).is_some() {
            continue;
        }
        let representation = legacy.representation;
        scores.upsert(RetrievalLaneScore::new(
            RetrievalScoreKind::LearnedSparse,
            i64::from(legacy.score_micros),
            RetrievalRawRank::unavailable(
                "legacy learned-sparse reason did not retain the backend rank",
            ),
            RetrievalScoreScale::unbounded("legacy_sparse_score_micros"),
            representation.clone(),
            RetrievalScoreFingerprint::new(
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
