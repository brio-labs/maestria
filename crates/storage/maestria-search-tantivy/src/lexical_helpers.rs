use crate::{
    keys::{card_key, chunk_key},
    scoring::descending_score,
};
use maestria_ports::{
    CardField, ChunkField, HitReason, IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit,
    LexicalChunkHit, LexicalHitMetadata, LexicalQuery, MatchMode, PortError, RetrieverIdentity,
};
use tantivy::query::{BooleanQuery, QueryParser, RegexQuery};

fn regex_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                vec!['\\', character]
            }
            character => vec![character],
        })
        .collect()
}

pub(super) fn build_parsed_query<T>(
    index: &tantivy::Index,
    fields: &[(tantivy::schema::Field, f32, T)],
    trimmed: &str,
    mode: MatchMode,
    error_context: &'static str,
) -> Result<Box<dyn tantivy::query::Query>, PortError> {
    let mut parser_fields = Vec::new();
    for (field, _, _) in fields {
        if !parser_fields.contains(field) {
            parser_fields.push(*field);
        }
    }
    let mut parser = QueryParser::for_index(index, parser_fields.clone());
    for (field, boost, _) in fields {
        parser.set_field_boost(*field, *boost);
    }
    let parsed_query = match mode {
        MatchMode::Contains => parser.parse_query(trimmed),
        MatchMode::Exact => {
            let escaped = trimmed.replace('"', "\\\"");
            let exact_query = format!("\"{escaped}\"");
            parser.parse_query(&exact_query)
        }
    }
    .map_err(|error| PortError::InvalidInputContext {
        context: error_context,
        source: error.to_string(),
    })?;

    if mode == MatchMode::Contains && !trimmed.chars().any(char::is_whitespace) {
        let pattern = format!(".*{}.*", regex_escape(&trimmed.to_lowercase()));
        let mut fallback_queries: Vec<Box<dyn tantivy::query::Query>> = Vec::new();
        for field in parser_fields {
            fallback_queries.push(Box::new(
                RegexQuery::from_pattern(&pattern, field).map_err(|error| {
                    PortError::InvalidInputContext {
                        context: error_context,
                        source: error.to_string(),
                    }
                })?,
            ));
        }
        if !fallback_queries.is_empty() {
            return Ok(Box::new(BooleanQuery::union(
                std::iter::once(parsed_query)
                    .chain(fallback_queries)
                    .collect(),
            )));
        }
    }
    Ok(parsed_query)
}

/// A query needle pre-normalized once per search instead of per candidate.
pub(super) struct NormalizedNeedle {
    /// Quote-replaced needle used by substring matching.
    text: String,
    /// Original needle used by exact-equality matching.
    raw: String,
}

impl NormalizedNeedle {
    pub(super) fn new(needle: &str) -> Self {
        Self {
            text: needle.replace('"', " "),
            raw: needle.to_string(),
        }
    }

    fn terms(&self) -> impl Iterator<Item = &str> {
        self.text.split_whitespace()
    }

    fn raw(&self) -> &str {
        &self.raw
    }
}

fn contains_match(value: &str, needle: &NormalizedNeedle) -> bool {
    if value.is_ascii() && needle.text.is_ascii() {
        let normalized = value.to_ascii_lowercase();
        normalized.contains(&needle.text) || needle.terms().all(|term| normalized.contains(term))
    } else {
        let normalized = value.to_lowercase();
        normalized.contains(&needle.text) || needle.terms().all(|term| normalized.contains(term))
    }
}

fn field_label(field: &ChunkField) -> &'static str {
    match field {
        ChunkField::Text => "text",
        ChunkField::Path => "path",
        ChunkField::Filename => "filename",
        ChunkField::Symbol => "symbol",
        ChunkField::Id => "id",
    }
}

fn card_field_label(field: &CardField) -> &'static str {
    match field {
        CardField::Title => "title",
        CardField::Body => "body",
        CardField::Path => "path",
        CardField::Filename => "filename",
        CardField::Symbol => "symbol",
        CardField::Id => "id",
    }
}

pub(super) fn score_chunk(
    chunk: &IndexedLexicalChunk,
    query: &LexicalQuery<ChunkField>,
    needle: &NormalizedNeedle,
) -> Option<(f32, HitReason)> {
    let mut matched_field = None;
    let mut raw_score = 0.0;

    for f in &query.fields {
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
            ChunkField::Id => {
                let key = chunk_key(chunk.artifact_id, chunk.chunk_id);
                let matches = match query.mode {
                    MatchMode::Contains => key.contains(&needle.text),
                    MatchMode::Exact => key == needle.raw(),
                };
                if matches {
                    matched_field = Some("id".to_string());
                    raw_score += f.boost;
                }
                continue;
            }
        };

        if let Some(s) = val {
            let matches = match query.mode {
                MatchMode::Contains => contains_match(s, needle),
                MatchMode::Exact => *s == needle.raw(),
            };
            if matches {
                if matched_field.is_none() {
                    matched_field = Some(field_label(&f.field).to_string());
                }
                raw_score += (len.min(u32::MAX as usize) as f32) * f.boost;
            }
        }
    }

    matched_field.map(|field_name| {
        let reason = match query.mode {
            MatchMode::Exact => HitReason::ExactMatch { field: field_name },
            MatchMode::Contains => HitReason::FieldMatch { field: field_name },
        };
        (raw_score, reason)
    })
}

pub(super) fn score_card(
    card: &IndexedLexicalCard,
    query: &LexicalQuery<CardField>,
    needle: &NormalizedNeedle,
) -> Option<(f32, HitReason)> {
    let mut matched_field = None;
    let mut raw_score = 0.0;

    for f in &query.fields {
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
            CardField::Id => {
                let key = card_key(card.artifact_id, card.card_id);
                let matches = match query.mode {
                    MatchMode::Contains => key.contains(&needle.text),
                    MatchMode::Exact => key == needle.raw(),
                };
                if matches {
                    matched_field = Some("id".to_string());
                    raw_score += f.boost;
                }
                continue;
            }
        };

        if let Some(s) = val {
            let matches = match query.mode {
                MatchMode::Contains => contains_match(s, needle),
                MatchMode::Exact => *s == needle.raw(),
            };
            if matches {
                if matched_field.is_none() {
                    matched_field = Some(card_field_label(&f.field).to_string());
                }
                raw_score += (len.min(u32::MAX as usize) as f32) * f.boost;
            }
        }
    }

    matched_field.map(|field_name| {
        let reason = match query.mode {
            MatchMode::Exact => HitReason::ExactMatch { field: field_name },
            MatchMode::Contains => HitReason::FieldMatch { field: field_name },
        };
        (raw_score, reason)
    })
}

fn create_lexical_hit_metadata(score: f32, rank: u32, reason: HitReason) -> LexicalHitMetadata {
    LexicalHitMetadata {
        retriever: RetrieverIdentity {
            name: "maestria-search-tantivy",
            version: env!("CARGO_PKG_VERSION"),
        },
        raw_score: score,
        raw_rank: rank,
        reason,
        snapshot_id: None,
    }
}

type ScoredChunk = (f32, u64, u64, IndexedLexicalChunk, HitReason);
type ScoredCard = (f32, u64, u64, IndexedLexicalCard, HitReason);

fn order_scored<T>(
    left: &(f32, u64, u64, T, HitReason),
    right: &(f32, u64, u64, T, HitReason),
) -> std::cmp::Ordering {
    descending_score(left.0, right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.cmp(&right.2))
}

pub(super) fn page_chunk_hits(
    mut scored: Vec<ScoredChunk>,
    query: &LexicalQuery<ChunkField>,
) -> Vec<LexicalChunkHit> {
    let keep = query.offset.saturating_add(query.limit);
    if keep > 0 && scored.len() > keep {
        scored.select_nth_unstable_by(keep - 1, order_scored);
        scored.truncate(keep);
    }
    scored.sort_by(order_scored);
    scored
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .enumerate()
        .map(|(rank, (score, _, _, chunk, reason))| LexicalChunkHit {
            chunk,
            metadata: create_lexical_hit_metadata(score, (query.offset + rank + 1) as u32, reason),
        })
        .collect()
}

pub(super) fn page_card_hits(
    mut scored: Vec<ScoredCard>,
    query: &LexicalQuery<CardField>,
) -> Vec<LexicalCardHit> {
    let keep = query.offset.saturating_add(query.limit);
    if keep > 0 && scored.len() > keep {
        scored.select_nth_unstable_by(keep - 1, order_scored);
        scored.truncate(keep);
    }
    scored.sort_by(order_scored);
    scored
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .enumerate()
        .map(|(rank, (score, _, _, card, reason))| LexicalCardHit {
            card,
            metadata: create_lexical_hit_metadata(score, (query.offset + rank + 1) as u32, reason),
        })
        .collect()
}
