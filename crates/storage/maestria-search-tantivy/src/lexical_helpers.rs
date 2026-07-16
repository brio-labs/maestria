use super::{card_key, chunk_key, descending_score};
use maestria_ports::{
    CardField, ChunkField, HitReason, IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit,
    LexicalChunkHit, LexicalHitMetadata, LexicalQuery, MatchMode, PortError, RetrieverIdentity,
};
use tantivy::query::{BooleanQuery, QueryParser, RegexQuery};

pub(super) const MAX_LEXICAL_CANDIDATES: usize = 10_000;

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
    error_context: &str,
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
    .map_err(|error| PortError::InvalidInput {
        message: format!("invalid {error_context}: {error}"),
    })?;

    if mode == MatchMode::Contains && !trimmed.chars().any(char::is_whitespace) {
        let pattern = format!(".*{}.*", regex_escape(&trimmed.to_lowercase()));
        let mut fallback_queries: Vec<Box<dyn tantivy::query::Query>> = Vec::new();
        for field in parser_fields {
            fallback_queries.push(Box::new(
                RegexQuery::from_pattern(&pattern, field).map_err(|error| {
                    PortError::InvalidInput {
                        message: format!("invalid {error_context}: {error}"),
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

fn contains_match(value: &str, needle: &str) -> bool {
    let normalized = value.to_lowercase();
    let normalized_needle = needle.replace('"', " ");
    normalized.contains(&normalized_needle)
        || normalized_needle
            .split_whitespace()
            .all(|term| normalized.contains(term))
}

pub(super) fn score_chunk(
    chunk: &IndexedLexicalChunk,
    query: &LexicalQuery<ChunkField>,
    needle: &str,
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
                    MatchMode::Contains => key.to_lowercase().contains(needle),
                    MatchMode::Exact => key == needle,
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
                MatchMode::Exact => *s == needle,
            };
            if matches {
                if matched_field.is_none() {
                    matched_field = Some(format!("{:?}", f.field).to_lowercase());
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
    needle: &str,
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
                    MatchMode::Contains => key.to_lowercase().contains(needle),
                    MatchMode::Exact => key == needle,
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
                MatchMode::Exact => *s == needle,
            };
            if matches {
                if matched_field.is_none() {
                    matched_field = Some(format!("{:?}", f.field).to_lowercase());
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
            name: "maestria-search-tantivy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        raw_score: score,
        raw_rank: rank,
        reason,
        snapshot_id: None,
    }
}

type ScoredChunk = (f32, u64, u64, IndexedLexicalChunk, HitReason);
type ScoredCard = (f32, u64, u64, IndexedLexicalCard, HitReason);

pub(super) fn page_chunk_hits(
    mut scored: Vec<ScoredChunk>,
    query: &LexicalQuery<ChunkField>,
) -> Vec<LexicalChunkHit> {
    scored.sort_by(|a, b| {
        descending_score(a.0, b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
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
    scored.sort_by(|a, b| {
        descending_score(a.0, b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
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
