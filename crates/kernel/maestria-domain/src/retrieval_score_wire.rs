use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{RepresentationName, RetrievalModelFingerprint, SearchCompatibilityError};

use super::{
    RetrievalLaneScore, RetrievalRawRank, RetrievalScoreFingerprint, RetrievalScoreKind,
    RetrievalScoreScale, RetrievalScoreSet,
};

/// Current-schema wire shape: `schema_version` plus typed lanes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CurrentScoreSetDto {
    pub(super) schema_version: u16,
    pub(super) lanes: Vec<RetrievalLaneScore>,
}

/// Legacy pre-versioning wire shape: two flat integer scores.
///
/// Kept as a read-only compatibility DTO: domain-event payload bytes are
/// immutable, so old rows are upcast in memory through this shape instead
/// of being rewritten.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyScoreSetDto {
    pub(super) bm25: u32,
    pub(super) semantic_similarity: u32,
}

/// Untagged dispatch between the current and legacy wire shapes.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum ScoreSetWire {
    Current(CurrentScoreSetDto),
    Legacy(LegacyScoreSetDto),
}

/// Upcast a legacy `{bm25, semantic_similarity}` payload into the canonical
/// versioned score set. Zero-valued legacy lanes are dropped, and the raw
/// backend rank is not retained by the old payload shape, so migrated lanes
/// carry an explicit `Unavailable` rank with a migration fingerprint.
pub(super) fn migrate_legacy_scores(
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
