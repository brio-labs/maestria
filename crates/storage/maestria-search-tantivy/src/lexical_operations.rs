use crate::execution::{Meter, validate_limit};
use crate::lexical_helpers::build_parsed_query;
use crate::search_helpers::{BoundedCollection, collect_bounded};
use crate::tantivy_index::{TantivyFullTextIndex, card_key, chunk_key, to_port_error};
use maestria_domain::SearchExecutionCompletion;
use maestria_ports::{
    BoundedSearch, CardField, ChunkField, IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit,
    LexicalChunkHit, LexicalQuery, MatchMode, PortError,
};
use tantivy::{
    Term,
    query::{BooleanQuery, TermSetQuery},
};

impl TantivyFullTextIndex {
    pub(crate) fn do_index_lexical_chunks(
        &self,
        chunks: Vec<IndexedLexicalChunk>,
    ) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "index lexical chunks requires a writable full-text index",
                source: "full-text index is read-only".to_string(),
            })?;
        for chunk in chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(chunk.artifact_id, chunk.chunk_id),
            ));
            writer
                .add_document(self.lexical_chunk_document(&chunk))
                .map_err(to_port_error)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    pub(crate) fn do_index_lexical_cards(
        &self,
        cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "index lexical cards requires a writable full-text index",
                source: "full-text index is read-only".to_string(),
            })?;
        for card in cards {
            writer.delete_term(Term::from_field_text(
                self.fields.card_key,
                &card_key(card.artifact_id, card.card_id),
            ));
            writer
                .add_document(self.lexical_card_document(&card))
                .map_err(to_port_error)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    pub(crate) fn do_search_lexical(
        &self,
        query: LexicalQuery<ChunkField>,
    ) -> Result<BoundedSearch<LexicalChunkHit>, PortError> {
        self.do_search_lexical_filtered(query, None)
    }

    pub(crate) fn do_search_lexical_filtered(
        &self,
        query: LexicalQuery<ChunkField>,
        filter: Option<
            &dyn Fn(
                maestria_domain::ChunkId,
                maestria_domain::ArtifactId,
            ) -> Result<bool, PortError>,
        >,
    ) -> Result<BoundedSearch<LexicalChunkHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty lexical chunk search query",
                source: "query must contain non-whitespace text".to_string(),
            });
        }
        if query.fields.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "lexical chunk search fields are empty",
                source: "at least one field is required".to_string(),
            });
        }
        validate_limit(
            query.limit,
            query.execution_budget,
            "lexical chunk result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        let fields = query
            .fields
            .iter()
            .map(|selector| {
                let field = match selector.field {
                    ChunkField::Text => self.fields.text,
                    ChunkField::Path => self.fields.path,
                    ChunkField::Filename => self.fields.filename,
                    ChunkField::Symbol => self.fields.symbol,
                    ChunkField::Id => self.fields.key,
                };
                (field, selector.boost, selector.field)
            })
            .collect::<Vec<_>>();
        let searcher = self.reader.searcher();
        let (allowed, authorization_stop) = if let Some(f) = filter {
            let (keys, stop) = self.allowed_chunk_keys(&searcher, f, &mut meter)?;
            if keys.is_empty() {
                if let Some(resource) = stop {
                    return Ok(
                        meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource))
                    );
                }
                return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
            }
            (Some(keys), stop)
        } else {
            (None, None)
        };
        let mut parsed_query =
            build_parsed_query(&self.index, &fields, trimmed, query.mode, "lexical query")?;
        if let Some(keys) = allowed {
            parsed_query = Box::new(BooleanQuery::intersection(vec![
                parsed_query,
                Box::new(TermSetQuery::new(
                    keys.into_iter()
                        .map(|key| Term::from_field_text(self.fields.key, &key)),
                )),
            ]));
        }
        let candidate_limit = remaining_candidate_limit(query.execution_budget, &meter);
        if candidate_limit == 0 {
            return Ok(meter.done(
                Vec::new(),
                SearchExecutionCompletion::Exhausted(
                    maestria_domain::SearchExecutionResource::Candidates,
                ),
            ));
        }
        let collection =
            collect_lexical_candidates(&searcher, &parsed_query, candidate_limit, &mut meter)?;
        if let Some(resource) = collection.stopped {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource)));
        }
        let needle = match query.mode {
            MatchMode::Contains => trimmed.to_lowercase(),
            MatchMode::Exact => trimmed.to_string(),
        };
        let (scored, score_stop) = self.score_lexical_chunks(
            &searcher,
            collection.docs,
            collection.truncated,
            &query,
            &needle,
            &mut meter,
        )?;
        Ok(self.finish_chunk_search(scored, &query, meter, authorization_stop.or(score_stop)))
    }
    pub(crate) fn do_search_cards_lexical(
        &self,
        query: LexicalQuery<CardField>,
    ) -> Result<BoundedSearch<LexicalCardHit>, PortError> {
        self.do_search_cards_lexical_filtered(query, None)
    }

    pub(crate) fn do_search_cards_lexical_filtered(
        &self,
        query: LexicalQuery<CardField>,
        filter: Option<
            &dyn Fn(
                maestria_domain::CardId,
                maestria_domain::ArtifactId,
            ) -> Result<bool, PortError>,
        >,
    ) -> Result<BoundedSearch<LexicalCardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "empty lexical card search query",
                source: "query must contain non-whitespace text".to_string(),
            });
        }
        if query.fields.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "lexical card search fields are empty",
                source: "at least one field is required".to_string(),
            });
        }
        validate_limit(
            query.limit,
            query.execution_budget,
            "lexical card result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
        }
        let fields = query
            .fields
            .iter()
            .map(|selector| {
                let field = match selector.field {
                    CardField::Title => self.fields.card_title,
                    CardField::Body => self.fields.card_body,
                    CardField::Path => self.fields.card_path,
                    CardField::Filename => self.fields.card_filename,
                    CardField::Symbol => self.fields.card_symbol,
                    CardField::Id => self.fields.card_key,
                };
                (field, selector.boost, selector.field)
            })
            .collect::<Vec<_>>();
        let searcher = self.reader.searcher();
        let (allowed, authorization_stop) = if let Some(f) = filter {
            let (keys, stop) = self.allowed_card_keys(&searcher, f, &mut meter)?;
            if keys.is_empty() {
                if let Some(resource) = stop {
                    return Ok(
                        meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource))
                    );
                }
                return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Complete));
            }
            (Some(keys), stop)
        } else {
            (None, None)
        };
        let mut parsed_query = build_parsed_query(
            &self.index,
            &fields,
            trimmed,
            query.mode,
            "lexical card query",
        )?;
        if let Some(keys) = allowed {
            parsed_query = Box::new(BooleanQuery::intersection(vec![
                parsed_query,
                Box::new(TermSetQuery::new(
                    keys.into_iter()
                        .map(|key| Term::from_field_text(self.fields.card_key, &key)),
                )),
            ]));
        }
        let candidate_limit = remaining_candidate_limit(query.execution_budget, &meter);
        if candidate_limit == 0 {
            return Ok(meter.done(
                Vec::new(),
                SearchExecutionCompletion::Exhausted(
                    maestria_domain::SearchExecutionResource::Candidates,
                ),
            ));
        }
        let collection =
            collect_lexical_candidates(&searcher, &parsed_query, candidate_limit, &mut meter)?;
        if let Some(resource) = collection.stopped {
            return Ok(meter.done(Vec::new(), SearchExecutionCompletion::Exhausted(resource)));
        }
        let needle = if query.mode == MatchMode::Contains {
            trimmed.to_lowercase()
        } else {
            trimmed.to_string()
        };
        let (scored, score_stop) = self.score_lexical_cards(
            &searcher,
            collection.docs,
            collection.truncated,
            &query,
            &needle,
            &mut meter,
        )?;
        Ok(self.finish_card_search(scored, &query, meter, authorization_stop.or(score_stop)))
    }
}
fn collect_lexical_candidates(
    searcher: &tantivy::Searcher,
    query: &dyn tantivy::query::Query,
    candidate_limit: usize,
    meter: &mut Meter,
) -> Result<BoundedCollection, PortError> {
    collect_bounded(searcher, query, 0, candidate_limit, candidate_limit, meter)
}
fn remaining_candidate_limit(
    budget: maestria_domain::SearchExecutionBudget,
    meter: &Meter,
) -> usize {
    maestria_domain::saturating_usize(
        budget
            .max_candidates()
            .saturating_sub(meter.usage.candidates),
    )
}
