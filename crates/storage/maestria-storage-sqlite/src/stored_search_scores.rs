//! DTO mirrors of the maestria-domain retrieval *score* subtree.
//!
//! The stored row owns its own wire format: every `Stored*` type here is a
//! serde shape independent of `maestria_domain`, with infallible
//! `from_domain` encoding and validated, fallible `try_into_domain` decoding.
//! No legacy wire shapes are preserved. These types are re-exported from
//! `crate::payloads::stored_search_outcome` (the outcome facade) and from
//! `crate::payloads::stored_search`.

use std::collections::BTreeMap;

use maestria_domain::{
    LearnedSparseContribution, LearnedSparseReason, RETRIEVAL_SCORE_SCHEMA_VERSION,
    RepresentationName, RetrievalLaneScore, RetrievalRawRank, RetrievalReason,
    RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search_plan::StoredRetrievalModelFingerprint;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRetrievalScoreKind {
    Exact,
    LexicalBm25,
    DenseSimilarity,
    LearnedSparse,
    LateInteraction,
    Graph,
    SpecializedRetrieval { route: String },
}

impl StoredRetrievalScoreKind {
    pub(crate) fn from_domain(value: &RetrievalScoreKind) -> Self {
        match value {
            RetrievalScoreKind::Exact => Self::Exact,
            RetrievalScoreKind::LexicalBm25 => Self::LexicalBm25,
            RetrievalScoreKind::DenseSimilarity => Self::DenseSimilarity,
            RetrievalScoreKind::LearnedSparse => Self::LearnedSparse,
            RetrievalScoreKind::LateInteraction => Self::LateInteraction,
            RetrievalScoreKind::Graph => Self::Graph,
            RetrievalScoreKind::SpecializedRetrieval { route } => Self::SpecializedRetrieval {
                route: route.clone(),
            },
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalScoreKind, PortError> {
        Ok(match self {
            Self::Exact => RetrievalScoreKind::Exact,
            Self::LexicalBm25 => RetrievalScoreKind::LexicalBm25,
            Self::DenseSimilarity => RetrievalScoreKind::DenseSimilarity,
            Self::LearnedSparse => RetrievalScoreKind::LearnedSparse,
            Self::LateInteraction => RetrievalScoreKind::LateInteraction,
            Self::Graph => RetrievalScoreKind::Graph,
            Self::SpecializedRetrieval { route } => {
                RetrievalScoreKind::SpecializedRetrieval { route }
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum StoredRetrievalRawRank {
    Ranked { rank: u32 },
    Unavailable { reason: String },
}

impl StoredRetrievalRawRank {
    pub(crate) fn from_domain(value: &RetrievalRawRank) -> Self {
        match value {
            RetrievalRawRank::Ranked { rank } => Self::Ranked { rank: *rank },
            RetrievalRawRank::Unavailable { reason } => Self::Unavailable {
                reason: reason.clone(),
            },
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalRawRank, PortError> {
        Ok(match self {
            Self::Ranked { rank } => RetrievalRawRank::Ranked { rank },
            Self::Unavailable { reason } => RetrievalRawRank::Unavailable { reason },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum StoredRetrievalScoreScale {
    Binary,
    Unbounded {
        name: String,
        higher_is_better: bool,
    },
    FixedPoint {
        name: String,
        denominator: u32,
        minimum: Option<i64>,
        maximum: Option<i64>,
        higher_is_better: bool,
    },
    RankDerived {
        name: String,
        higher_is_better: bool,
    },
}

impl StoredRetrievalScoreScale {
    pub(crate) fn from_domain(value: &RetrievalScoreScale) -> Self {
        match value {
            RetrievalScoreScale::Binary => Self::Binary,
            RetrievalScoreScale::Unbounded {
                name,
                higher_is_better,
            } => Self::Unbounded {
                name: name.clone(),
                higher_is_better: *higher_is_better,
            },
            RetrievalScoreScale::FixedPoint {
                name,
                denominator,
                minimum,
                maximum,
                higher_is_better,
            } => Self::FixedPoint {
                name: name.clone(),
                denominator: *denominator,
                minimum: *minimum,
                maximum: *maximum,
                higher_is_better: *higher_is_better,
            },
            RetrievalScoreScale::RankDerived {
                name,
                higher_is_better,
            } => Self::RankDerived {
                name: name.clone(),
                higher_is_better: *higher_is_better,
            },
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalScoreScale, PortError> {
        Ok(match self {
            Self::Binary => RetrievalScoreScale::Binary,
            Self::Unbounded {
                name,
                higher_is_better,
            } => RetrievalScoreScale::Unbounded {
                name,
                higher_is_better,
            },
            Self::FixedPoint {
                name,
                denominator,
                minimum,
                maximum,
                higher_is_better,
            } => RetrievalScoreScale::FixedPoint {
                name,
                denominator,
                minimum,
                maximum,
                higher_is_better,
            },
            Self::RankDerived {
                name,
                higher_is_better,
            } => RetrievalScoreScale::RankDerived {
                name,
                higher_is_better,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredRetrievalScoreFingerprint {
    pub(crate) identity: StoredRetrievalModelFingerprint,
    pub(crate) components: BTreeMap<String, String>,
}

impl StoredRetrievalScoreFingerprint {
    pub(crate) fn from_domain(value: &RetrievalScoreFingerprint) -> Self {
        Self {
            identity: StoredRetrievalModelFingerprint::from_domain(&value.identity),
            components: value.components.clone(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalScoreFingerprint, PortError> {
        Ok(RetrievalScoreFingerprint::new(
            self.identity.try_into_domain()?,
            self.components,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredRetrievalLaneScore {
    pub(crate) score_kind: StoredRetrievalScoreKind,
    pub(crate) raw_score: i64,
    pub(crate) raw_rank: StoredRetrievalRawRank,
    pub(crate) scale: StoredRetrievalScoreScale,
    pub(crate) representation: String,
    pub(crate) fingerprint: StoredRetrievalScoreFingerprint,
}

impl StoredRetrievalLaneScore {
    pub(crate) fn from_domain(value: &RetrievalLaneScore) -> Self {
        Self {
            score_kind: StoredRetrievalScoreKind::from_domain(&value.score_kind),
            raw_score: value.raw_score,
            raw_rank: StoredRetrievalRawRank::from_domain(&value.raw_rank),
            scale: StoredRetrievalScoreScale::from_domain(&value.scale),
            representation: value.representation.0.clone(),
            fingerprint: StoredRetrievalScoreFingerprint::from_domain(&value.fingerprint),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalLaneScore, PortError> {
        Ok(RetrievalLaneScore::new(
            self.score_kind.try_into_domain()?,
            self.raw_score,
            self.raw_rank.try_into_domain()?,
            self.scale.try_into_domain()?,
            RepresentationName::new(self.representation),
            self.fingerprint.try_into_domain()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredRetrievalScoreSet {
    pub(crate) schema_version: u16,
    pub(crate) lanes: Vec<StoredRetrievalLaneScore>,
}

impl StoredRetrievalScoreSet {
    pub(crate) fn from_domain(value: &RetrievalScoreSet) -> Self {
        Self {
            schema_version: value.schema_version(),
            lanes: value
                .lanes()
                .iter()
                .map(StoredRetrievalLaneScore::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalScoreSet, PortError> {
        if self.schema_version != RETRIEVAL_SCORE_SCHEMA_VERSION {
            return Err(PortError::InvalidInputContext {
                context: "decode stored retrieval score set",
                source: format!(
                    "unsupported retrieval score schema version {}",
                    self.schema_version
                ),
            });
        }
        let lanes = self
            .lanes
            .into_iter()
            .map(StoredRetrievalLaneScore::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        RetrievalScoreSet::new(lanes).map_err(|error| PortError::InvalidInputContext {
            context: "decode stored retrieval score set",
            source: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredLearnedSparseContribution {
    pub(crate) term_id: u32,
    pub(crate) contribution_micros: u32,
}

impl StoredLearnedSparseContribution {
    pub(crate) fn from_domain(value: &LearnedSparseContribution) -> Self {
        Self {
            term_id: value.term_id,
            contribution_micros: value.contribution_micros,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<LearnedSparseContribution, PortError> {
        Ok(LearnedSparseContribution {
            term_id: self.term_id,
            contribution_micros: self.contribution_micros,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredLearnedSparseReason {
    pub(crate) contributions: Vec<StoredLearnedSparseContribution>,
}

impl StoredLearnedSparseReason {
    pub(crate) fn from_domain(value: &LearnedSparseReason) -> Self {
        Self {
            contributions: value
                .contributions
                .iter()
                .map(StoredLearnedSparseContribution::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<LearnedSparseReason, PortError> {
        let contributions = self
            .contributions
            .into_iter()
            .map(StoredLearnedSparseContribution::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LearnedSparseReason::new(contributions))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRetrievalReason {
    ExactMatch,
    LexicalMatch,
    SemanticSimilarity,
    CitationLink,
    GraphTraversal,
    LateInteraction,
    SpecializedRetrieval { route: String },
    LearnedSparse(Box<StoredLearnedSparseReason>),
}

impl StoredRetrievalReason {
    pub(crate) fn from_domain(value: &RetrievalReason) -> Self {
        match value {
            RetrievalReason::ExactMatch => Self::ExactMatch,
            RetrievalReason::LexicalMatch => Self::LexicalMatch,
            RetrievalReason::SemanticSimilarity => Self::SemanticSimilarity,
            RetrievalReason::CitationLink => Self::CitationLink,
            RetrievalReason::GraphTraversal => Self::GraphTraversal,
            RetrievalReason::LateInteraction => Self::LateInteraction,
            RetrievalReason::SpecializedRetrieval { route } => Self::SpecializedRetrieval {
                route: route.clone(),
            },
            RetrievalReason::LearnedSparse(reason) => {
                Self::LearnedSparse(Box::new(StoredLearnedSparseReason::from_domain(reason)))
            }
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalReason, PortError> {
        Ok(match self {
            Self::ExactMatch => RetrievalReason::ExactMatch,
            Self::LexicalMatch => RetrievalReason::LexicalMatch,
            Self::SemanticSimilarity => RetrievalReason::SemanticSimilarity,
            Self::CitationLink => RetrievalReason::CitationLink,
            Self::GraphTraversal => RetrievalReason::GraphTraversal,
            Self::LateInteraction => RetrievalReason::LateInteraction,
            Self::SpecializedRetrieval { route } => RetrievalReason::SpecializedRetrieval { route },
            Self::LearnedSparse(reason) => {
                RetrievalReason::LearnedSparse(Box::new(reason.try_into_domain()?))
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use maestria_domain::{
        RepresentationName, RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank,
        RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet,
    };
    use maestria_ports::PortError;

    use super::*;

    fn sample_score_set() -> Result<RetrievalScoreSet, Box<dyn std::error::Error>> {
        let lane = RetrievalLaneScore::new(
            RetrievalScoreKind::Exact,
            1,
            RetrievalRawRank::ranked(1),
            RetrievalScoreScale::Binary,
            RepresentationName::new("text/plain"),
            RetrievalScoreFingerprint::new(
                RetrievalModelFingerprint::new("fp-v1".to_string())?,
                BTreeMap::from([("model".to_string(), "exact".to_string())]),
            ),
        );
        Ok(RetrievalScoreSet::new(vec![lane])?)
    }

    #[test]
    fn score_set_with_unknown_schema_version_is_rejected() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut stored = StoredRetrievalScoreSet::from_domain(&sample_score_set()?);
        stored.schema_version = 999;
        assert!(matches!(
            stored.try_into_domain(),
            Err(PortError::InvalidInputContext { .. })
        ));
        Ok(())
    }
}
