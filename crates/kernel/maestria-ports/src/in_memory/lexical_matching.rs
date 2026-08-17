//! Lexical field matching: query preparation, per-field containment/exact
//! predicates, and hit-metadata construction shared by every in-memory
//! lexical lane (Rule 16: cross-lane behavior crosses typed functions).

use crate::PortError;
use crate::lexical::{FieldSelector, HitReason, LexicalHitMetadata, MatchMode, RetrieverIdentity};

pub(super) fn validate_and_prepare_query(
    q: &str,
    mode: MatchMode,
    err_msg: &str,
) -> Result<String, PortError> {
    if q.trim().is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "lexical query is empty",
            source: err_msg.to_string(),
        });
    }
    Ok(match mode {
        MatchMode::Contains => q.trim().to_lowercase(),
        MatchMode::Exact => q.trim().to_string(),
    })
}

pub(super) fn contains_match(value: &str, needle: &str) -> bool {
    let normalized = value.to_lowercase();
    let normalized_needle = needle.replace('"', " ");
    normalized.contains(&normalized_needle)
        || normalized_needle
            .split_whitespace()
            .all(|term| normalized.contains(term))
}

pub(super) fn process_field_match<F: std::fmt::Debug>(
    val: Option<&String>,
    len: usize,
    f: &FieldSelector<F>,
    mode: MatchMode,
    needle: &str,
    matched_field: &mut Option<String>,
    raw_score: &mut f32,
) {
    if let Some(s) = val {
        let matches = match mode {
            MatchMode::Contains => contains_match(s, needle),
            MatchMode::Exact => s == needle,
        };
        if matches {
            if matched_field.is_none() {
                *matched_field = Some(format!("{:?}", f.field).to_lowercase());
            }
            *raw_score += (len.min(u32::MAX as usize) as f32) * f.boost;
        }
    }
}

pub(super) fn build_metadata(
    matched_field: Option<String>,
    mode: MatchMode,
    raw_score: f32,
) -> Option<LexicalHitMetadata> {
    matched_field.map(|field| LexicalHitMetadata {
        retriever: RetrieverIdentity {
            name: "InMemoryFullText",
            version: "1.0",
        },
        raw_score,
        raw_rank: 0,
        reason: match mode {
            MatchMode::Exact => HitReason::ExactMatch { field },
            MatchMode::Contains => HitReason::FieldMatch { field },
        },
        snapshot_id: None,
    })
}
