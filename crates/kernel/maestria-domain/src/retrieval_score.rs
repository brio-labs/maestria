use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::{RetrievalModelFingerprint, SearchCompatibilityError};
use crate::RepresentationName;

/// Canonical retrieval score provenance schema.
pub const RETRIEVAL_SCORE_SCHEMA_VERSION: u16 = 2;

/// The semantic meaning of a lane's raw score.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalScoreKind {
    Exact,
    LexicalBm25,
    DenseSimilarity,
    LearnedSparse,
    LateInteraction,
    Graph,
    SpecializedRetrieval { route: String },
}

impl RetrievalScoreKind {
    fn validation_key(&self) -> String {
        match self {
            Self::Exact => "exact".to_string(),
            Self::LexicalBm25 => "lexical_bm25".to_string(),
            Self::DenseSimilarity => "dense_similarity".to_string(),
            Self::LearnedSparse => "learned_sparse".to_string(),
            Self::LateInteraction => "late_interaction".to_string(),
            Self::Graph => "graph".to_string(),
            Self::SpecializedRetrieval { route } => format!("specialized_retrieval:{route}"),
        }
    }

    fn validate(&self) -> Result<(), SearchCompatibilityError> {
        if let Self::SpecializedRetrieval { route } = self
            && route.trim().is_empty()
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "specialized retrieval route must not be empty",
            ));
        }
        Ok(())
    }
}

/// The raw rank emitted by a backend, or an explicit reason it is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RetrievalRawRank {
    Ranked { rank: u32 },
    Unavailable { reason: String },
}

impl RetrievalRawRank {
    pub fn ranked(rank: u32) -> Self {
        Self::Ranked { rank }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    fn validate(&self) -> Result<(), SearchCompatibilityError> {
        match self {
            Self::Ranked { rank: 0 } => Err(SearchCompatibilityError::InvalidScoreProvenance(
                "raw rank must be one-based",
            )),
            Self::Unavailable { reason } if reason.trim().is_empty() => {
                Err(SearchCompatibilityError::InvalidScoreProvenance(
                    "unavailable raw rank requires a reason",
                ))
            }
            _ => Ok(()),
        }
    }
}

/// Scale semantics for one homogeneous raw-score lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetrievalScoreScale {
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

impl RetrievalScoreScale {
    pub fn unbounded(name: impl Into<String>) -> Self {
        Self::Unbounded {
            name: name.into(),
            higher_is_better: true,
        }
    }

    pub fn fixed_point(name: impl Into<String>, denominator: u32) -> Self {
        Self::FixedPoint {
            name: name.into(),
            denominator,
            minimum: None,
            maximum: None,
            higher_is_better: true,
        }
    }

    pub fn bounded_fixed_point(
        name: impl Into<String>,
        denominator: u32,
        minimum: i64,
        maximum: i64,
    ) -> Self {
        Self::FixedPoint {
            name: name.into(),
            denominator,
            minimum: Some(minimum),
            maximum: Some(maximum),
            higher_is_better: true,
        }
    }

    pub fn rank_derived(name: impl Into<String>) -> Self {
        Self::RankDerived {
            name: name.into(),
            higher_is_better: true,
        }
    }

    fn validate(&self, raw_score: i64) -> Result<(), SearchCompatibilityError> {
        let (name, minimum, maximum) = match self {
            Self::Binary => {
                if !(0..=1).contains(&raw_score) {
                    return Err(SearchCompatibilityError::InvalidScoreProvenance(
                        "binary raw score must be zero or one",
                    ));
                }
                return Ok(());
            }
            Self::Unbounded { name, .. } | Self::RankDerived { name, .. } => (name, None, None),
            Self::FixedPoint {
                name,
                denominator,
                minimum,
                maximum,
                ..
            } => {
                if *denominator == 0 {
                    return Err(SearchCompatibilityError::InvalidScoreProvenance(
                        "fixed-point denominator must be positive",
                    ));
                }
                (name, *minimum, *maximum)
            }
        };
        if name.trim().is_empty() {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "score scale name must not be empty",
            ));
        }
        if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "score scale minimum must not exceed maximum",
            ));
        }
        if minimum.is_some_and(|min| raw_score < min) || maximum.is_some_and(|max| raw_score > max)
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "raw score is outside its declared scale",
            ));
        }
        Ok(())
    }
}

/// Complete identity and structured fingerprint components for one representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalScoreFingerprint {
    pub identity: RetrievalModelFingerprint,
    pub components: BTreeMap<String, String>,
}

impl RetrievalScoreFingerprint {
    pub fn new(identity: RetrievalModelFingerprint, components: BTreeMap<String, String>) -> Self {
        Self {
            identity,
            components,
        }
    }

    fn validate(&self) -> Result<(), SearchCompatibilityError> {
        if self
            .components
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "score fingerprint components must have non-empty keys and values",
            ));
        }
        Ok(())
    }
}

/// Raw score provenance emitted by exactly one retrieval lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalLaneScore {
    pub score_kind: RetrievalScoreKind,
    pub raw_score: i64,
    pub raw_rank: RetrievalRawRank,
    pub scale: RetrievalScoreScale,
    pub representation: RepresentationName,
    pub fingerprint: RetrievalScoreFingerprint,
}

impl RetrievalLaneScore {
    pub fn new(
        score_kind: RetrievalScoreKind,
        raw_score: i64,
        raw_rank: RetrievalRawRank,
        scale: RetrievalScoreScale,
        representation: RepresentationName,
        fingerprint: RetrievalScoreFingerprint,
    ) -> Self {
        Self {
            score_kind,
            raw_score,
            raw_rank,
            scale,
            representation,
            fingerprint,
        }
    }

    fn validate(&self) -> Result<(), SearchCompatibilityError> {
        self.score_kind.validate()?;
        self.raw_rank.validate()?;
        self.scale.validate(self.raw_score)?;
        if self.representation.0.trim().is_empty() {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "score representation must not be empty",
            ));
        }
        self.fingerprint.validate()
    }
}

/// Versioned, canonical collection of heterogeneous lane scores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalScoreSet {
    pub schema_version: u16,
    pub lanes: Vec<RetrievalLaneScore>,
}

impl Default for RetrievalScoreSet {
    fn default() -> Self {
        Self::empty()
    }
}

impl RetrievalScoreSet {
    pub fn empty() -> Self {
        Self {
            schema_version: RETRIEVAL_SCORE_SCHEMA_VERSION,
            lanes: Vec::new(),
        }
    }

    pub fn single(score: RetrievalLaneScore) -> Result<Self, SearchCompatibilityError> {
        Self::new(vec![score])
    }

    pub fn new(mut lanes: Vec<RetrievalLaneScore>) -> Result<Self, SearchCompatibilityError> {
        for lane in &lanes {
            lane.validate()?;
        }
        lanes.sort_by_key(|lane| lane.score_kind.validation_key());
        let mut seen = BTreeSet::new();
        if lanes
            .iter()
            .any(|lane| !seen.insert(lane.score_kind.validation_key()))
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "duplicate score kind in one candidate",
            ));
        }
        Ok(Self {
            schema_version: RETRIEVAL_SCORE_SCHEMA_VERSION,
            lanes,
        })
    }

    pub fn canonicalize(&mut self) -> Result<(), SearchCompatibilityError> {
        let canonical = Self::new(std::mem::take(&mut self.lanes))?;
        self.schema_version = canonical.schema_version;
        self.lanes = canonical.lanes;
        Ok(())
    }

    pub fn lane(&self, kind: &RetrievalScoreKind) -> Option<&RetrievalLaneScore> {
        self.lanes.iter().find(|lane| &lane.score_kind == kind)
    }

    pub fn upsert(&mut self, score: RetrievalLaneScore) -> Result<(), SearchCompatibilityError> {
        self.lanes
            .retain(|lane| lane.score_kind != score.score_kind);
        self.lanes.push(score);
        self.canonicalize()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentScoreSetDto {
    schema_version: u16,
    lanes: Vec<RetrievalLaneScore>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScoreSetDto {
    bm25: u32,
    semantic_similarity: u32,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ScoreSetWire {
    Current(CurrentScoreSetDto),
    Legacy(LegacyScoreSetDto),
}

impl<'de> Deserialize<'de> for RetrievalScoreSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match ScoreSetWire::deserialize(deserializer)? {
            ScoreSetWire::Current(dto) => {
                if dto.schema_version != RETRIEVAL_SCORE_SCHEMA_VERSION {
                    return Err(D::Error::custom(format!(
                        "unsupported retrieval score schema version {}",
                        dto.schema_version
                    )));
                }
                Self::new(dto.lanes).map_err(D::Error::custom)
            }
            ScoreSetWire::Legacy(dto) => migrate_legacy_scores(dto).map_err(D::Error::custom),
        }
    }
}

fn migrate_legacy_scores(
    legacy: LegacyScoreSetDto,
) -> Result<RetrievalScoreSet, SearchCompatibilityError> {
    let unavailable =
        || RetrievalRawRank::unavailable("legacy score payload did not retain the backend rank");
    let mut lanes = Vec::new();
    if legacy.bm25 != 0 {
        let representation = RepresentationName::new("lexical_text_v1");
        lanes.push(RetrievalLaneScore::new(
            RetrievalScoreKind::LexicalBm25,
            i64::from(legacy.bm25),
            unavailable(),
            RetrievalScoreScale::unbounded("legacy_bm25"),
            representation.clone(),
            RetrievalScoreFingerprint::new(
                RetrievalModelFingerprint::new("legacy:lexical-bm25:v1".to_string())?,
                BTreeMap::from([
                    ("migration".to_string(), "score_schema_v1_to_v2".to_string()),
                    ("representation".to_string(), representation.0),
                ]),
            ),
        ));
    }
    if legacy.semantic_similarity != 0 {
        let representation = RepresentationName::new("dense_text_v1");
        lanes.push(RetrievalLaneScore::new(
            RetrievalScoreKind::DenseSimilarity,
            i64::from(legacy.semantic_similarity),
            unavailable(),
            RetrievalScoreScale::bounded_fixed_point(
                "legacy_dense_similarity_micros",
                1_000_000,
                0,
                1_000_000,
            ),
            representation.clone(),
            RetrievalScoreFingerprint::new(
                RetrievalModelFingerprint::new("legacy:dense-similarity:v1".to_string())?,
                BTreeMap::from([
                    ("migration".to_string(), "score_schema_v1_to_v2".to_string()),
                    ("representation".to_string(), representation.0),
                ]),
            ),
        ));
    }
    RetrievalScoreSet::new(lanes)
}
