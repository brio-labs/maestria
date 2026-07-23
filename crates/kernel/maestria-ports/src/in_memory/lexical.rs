use std::sync::{Arc, Mutex};

use crate::PortError;
use crate::lexical::{
    CardField, ChunkField, HitReason, IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit,
    LexicalChunkHit, LexicalHitMetadata, LexicalQuery, MatchMode, RetrieverIdentity,
};
const MAX_LEXICAL_CANDIDATES: usize = 10_000;

fn validate_and_prepare_query(
    q: &str,
    mode: MatchMode,
    err_msg: &str,
) -> Result<String, PortError> {
    if q.trim().is_empty() {
        return Err(PortError::InvalidInput {
            message: err_msg.to_string(),
        });
    }
    Ok(match mode {
        MatchMode::Contains => q.trim().to_lowercase(),
        MatchMode::Exact => q.trim().to_string(),
    })
}

fn contains_match(value: &str, needle: &str) -> bool {
    let normalized = value.to_lowercase();
    let normalized_needle = needle.replace('"', " ");
    normalized.contains(&normalized_needle)
        || normalized_needle
            .split_whitespace()
            .all(|term| normalized.contains(term))
}

fn process_field_match<F: std::fmt::Debug>(
    val: Option<&String>,
    len: usize,
    f: &crate::lexical::FieldSelector<F>,
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

fn build_metadata(
    matched_field: Option<String>,
    mode: MatchMode,
    raw_score: f32,
) -> Option<LexicalHitMetadata> {
    matched_field.map(|field| LexicalHitMetadata {
        retriever: RetrieverIdentity {
            name: "InMemoryFullText".into(),
            version: "1.0".into(),
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

fn page_and_rank_hits<T, S, A, I, K1, K2>(
    mut hits: Vec<T>,
    offset: usize,
    limit: usize,
    score_fn: S,
    artifact_id_fn: A,
    item_id_fn: I,
    mut set_rank: impl FnMut(&mut T, u32),
) -> Vec<T>
where
    S: Fn(&T) -> f32,
    A: Fn(&T) -> K1,
    K1: Ord,
    I: Fn(&T) -> K2,
    K2: Ord,
{
    hits.sort_by(|a, b| {
        let score_order = match score_fn(b).partial_cmp(&score_fn(a)) {
            Some(ordering) => ordering,
            None => std::cmp::Ordering::Equal,
        };
        score_order
            .then_with(|| artifact_id_fn(a).cmp(&artifact_id_fn(b)))
            .then_with(|| item_id_fn(a).cmp(&item_id_fn(b)))
    });
    hits.into_iter()
        .enumerate()
        .skip(offset)
        .take(limit)
        .map(|(i, mut hit)| {
            set_rank(&mut hit, (i + 1) as u32);
            hit
        })
        .collect()
}

pub(crate) fn index_lexical_chunks(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    chunks: Vec<IndexedLexicalChunk>,
) -> Result<(), PortError> {
    let mut guard = lexical_chunks.lock().map_err(|_| PortError::Internal {
        message: "index lock poisoned".to_string(),
    })?;
    for chunk in &chunks {
        guard.retain(|existing| {
            existing.artifact_id != chunk.artifact_id || existing.chunk_id != chunk.chunk_id
        });
    }
    guard.extend(chunks);
    Ok(())
}

pub(crate) fn index_lexical_cards(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    cards: Vec<IndexedLexicalCard>,
) -> Result<(), PortError> {
    let mut guard = lexical_cards.lock().map_err(|_| PortError::Internal {
        message: "index lock poisoned".to_string(),
    })?;
    for card in &cards {
        guard.retain(|c| c.artifact_id != card.artifact_id || c.card_id != card.card_id);
    }
    guard.extend(cards);
    Ok(())
}

pub(crate) fn search_lexical(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    query: LexicalQuery<ChunkField>,
) -> Result<Vec<LexicalChunkHit>, PortError> {
    search_lexical_filtered(lexical_chunks, query, &|_, _| true)
}

pub(crate) fn search_lexical_filtered(
    lexical_chunks: &Arc<Mutex<Vec<IndexedLexicalChunk>>>,
    query: LexicalQuery<ChunkField>,
    filter: &dyn Fn(maestria_domain::ChunkId, maestria_domain::ArtifactId) -> bool,
) -> Result<Vec<LexicalChunkHit>, PortError> {
    let needle = validate_and_prepare_query(
        &query.q,
        query.mode,
        "lexical search query must not be empty",
    )?;
    if query.fields.is_empty() {
        return Err(PortError::InvalidInput {
            message: "lexical search requires at least one field".to_string(),
        });
    }
    let guard = lexical_chunks.lock().map_err(|_| PortError::Internal {
        message: "index lock poisoned".to_string(),
    })?;

    let hits = guard
        .iter()
        .filter_map(|chunk| {
            if !filter(chunk.chunk_id, chunk.artifact_id) {
                return None;
            }
            let mut matched_field = None;
            let mut raw_score = 0.0;

            for f in &query.fields {
                if let ChunkField::Id = f.field {
                    let key = format!("{}:{}", chunk.artifact_id.value(), chunk.chunk_id.value());
                    let matches = match query.mode {
                        MatchMode::Contains => key.contains(&needle),
                        MatchMode::Exact => key == needle,
                    };
                    if matches {
                        matched_field = Some("id".to_string());
                        raw_score += f.boost;
                    }
                    continue;
                }

                let (val, len) = match f.field {
                    ChunkField::Text => (Some(&chunk.text), chunk.text.len()),
                    ChunkField::Path => (
                        chunk.path.as_ref(),
                        chunk.path.as_ref().map_or(0, String::len),
                    ),
                    ChunkField::Filename => (
                        chunk.filename.as_ref(),
                        chunk.filename.as_ref().map_or(0, String::len),
                    ),
                    ChunkField::Symbol => (
                        chunk.symbol.as_ref(),
                        chunk.symbol.as_ref().map_or(0, String::len),
                    ),
                    ChunkField::Id => (None, 0),
                };

                process_field_match(
                    val,
                    len,
                    f,
                    query.mode,
                    &needle,
                    &mut matched_field,
                    &mut raw_score,
                );
            }

            build_metadata(matched_field, query.mode, raw_score).map(|metadata| LexicalChunkHit {
                chunk: chunk.clone(),
                metadata,
            })
        })
        .take(MAX_LEXICAL_CANDIDATES)
        .collect::<Vec<_>>();

    Ok(page_and_rank_hits(
        hits,
        query.offset,
        query.limit,
        |h| h.metadata.raw_score,
        |h| h.chunk.artifact_id,
        |h| h.chunk.chunk_id,
        |h, rank| h.metadata.raw_rank = rank,
    ))
}

pub(crate) fn search_cards_lexical(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    query: LexicalQuery<CardField>,
) -> Result<Vec<LexicalCardHit>, PortError> {
    search_cards_lexical_filtered(lexical_cards, query, &|_, _| true)
}

pub(crate) fn search_cards_lexical_filtered(
    lexical_cards: &Arc<Mutex<Vec<IndexedLexicalCard>>>,
    query: LexicalQuery<CardField>,
    filter: &dyn Fn(maestria_domain::CardId, maestria_domain::ArtifactId) -> bool,
) -> Result<Vec<LexicalCardHit>, PortError> {
    let needle = validate_and_prepare_query(
        &query.q,
        query.mode,
        "lexical card search query must not be empty",
    )?;
    if query.fields.is_empty() {
        return Err(PortError::InvalidInput {
            message: "lexical card search requires at least one field".to_string(),
        });
    }
    let guard = lexical_cards.lock().map_err(|_| PortError::Internal {
        message: "index lock poisoned".to_string(),
    })?;

    let hits = guard
        .iter()
        .filter_map(|card| {
            if !filter(card.card_id, card.artifact_id) {
                return None;
            }
            let mut matched_field = None;
            let mut raw_score = 0.0;

            for f in &query.fields {
                if let CardField::Id = f.field {
                    let key = format!("card:{}:{}", card.artifact_id.value(), card.card_id.value());
                    let matches = match query.mode {
                        MatchMode::Contains => key.contains(&needle),
                        MatchMode::Exact => key == needle,
                    };
                    if matches {
                        matched_field = Some("id".to_string());
                        raw_score += f.boost;
                    }
                    continue;
                }

                let (val, len) = match f.field {
                    CardField::Title => (Some(&card.title), card.title.len()),
                    CardField::Body => (Some(&card.body), card.body.len()),
                    CardField::Path => (
                        card.path.as_ref(),
                        card.path.as_ref().map_or(0, String::len),
                    ),
                    CardField::Filename => (
                        card.filename.as_ref(),
                        card.filename.as_ref().map_or(0, String::len),
                    ),
                    CardField::Symbol => (
                        card.symbol.as_ref(),
                        card.symbol.as_ref().map_or(0, String::len),
                    ),
                    CardField::Id => (None, 0),
                };

                process_field_match(
                    val,
                    len,
                    f,
                    query.mode,
                    &needle,
                    &mut matched_field,
                    &mut raw_score,
                );
            }

            build_metadata(matched_field, query.mode, raw_score).map(|metadata| LexicalCardHit {
                card: card.clone(),
                metadata,
            })
        })
        .take(MAX_LEXICAL_CANDIDATES)
        .collect::<Vec<_>>();

    Ok(page_and_rank_hits(
        hits,
        query.offset,
        query.limit,
        |h| h.metadata.raw_score,
        |h| h.card.artifact_id,
        |h| h.card.card_id,
        |h, rank| h.metadata.raw_rank = rank,
    ))
}

#[cfg(test)]
#[path = "lexical_tests.rs"]
mod tests;
