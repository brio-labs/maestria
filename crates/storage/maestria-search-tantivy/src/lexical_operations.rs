use super::lexical_helpers::{
    MAX_LEXICAL_CANDIDATES, build_parsed_query, page_card_hits, page_chunk_hits, score_card,
    score_chunk,
};
use super::{TantivyFullTextIndex, card_key, chunk_key, to_port_error};
use maestria_ports::{
    CardField, ChunkField, IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit,
    LexicalChunkHit, LexicalQuery, MatchMode, PortError,
};
use tantivy::{
    TantivyDocument, Term,
    collector::TopDocs,
    query::{BooleanQuery, TermSetQuery},
};

impl TantivyFullTextIndex {
    pub(crate) fn do_index_lexical_chunks(
        &self,
        chunks: Vec<IndexedLexicalChunk>,
    ) -> Result<(), PortError> {
        let mut writer = self.writer.lock().map_err(|_| PortError::Internal {
            message: "tantivy writer lock poisoned".to_string(),
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
        let mut writer = self.writer.lock().map_err(|_| PortError::Internal {
            message: "tantivy writer lock poisoned".to_string(),
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
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        self.do_search_lexical_filtered(query, None)
    }

    pub(crate) fn do_search_lexical_filtered(
        &self,
        query: LexicalQuery<ChunkField>,
        filter: Option<&dyn Fn(maestria_domain::ChunkId, maestria_domain::ArtifactId) -> bool>,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "lexical search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        if query.fields.is_empty() {
            return Err(PortError::InvalidInput {
                message: "lexical search requires at least one field".to_string(),
            });
        }
        let mut fields = Vec::new();

        for selector in &query.fields {
            let field = match selector.field {
                ChunkField::Text => self.fields.text,
                ChunkField::Path => self.fields.path,
                ChunkField::Filename => self.fields.filename,
                ChunkField::Symbol => self.fields.symbol,
                ChunkField::Id => self.fields.key,
            };
            fields.push((field, selector.boost, selector.field));
        }

        let searcher = self.reader.searcher();
        let allowed = if let Some(f) = filter {
            let keys = self.allowed_chunk_keys(&searcher, f)?;
            if keys.is_empty() {
                return Ok(Vec::new());
            }
            Some(keys)
        } else {
            None
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
        let candidate_limit = MAX_LEXICAL_CANDIDATES;
        let top_docs = searcher
            .search(
                &parsed_query,
                &TopDocs::with_limit(candidate_limit).order_by_score(),
            )
            .map_err(to_port_error)?;
        let mut scored = Vec::new();

        let needle = match query.mode {
            MatchMode::Contains => trimmed.to_lowercase(),
            MatchMode::Exact => trimmed.to_string(),
        };

        for (_, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            if document.get_first(self.fields.chunk_id).is_none() {
                continue;
            }
            let chunk = self.read_lexical_chunk(&document)?;

            if let Some((raw_score, reason)) = score_chunk(&chunk, &query, &needle) {
                scored.push((
                    raw_score,
                    chunk.artifact_id.value(),
                    chunk.chunk_id.value(),
                    chunk,
                    reason,
                ));
            }
        }
        Ok(page_chunk_hits(scored, &query))
    }

    pub(crate) fn do_search_cards_lexical(
        &self,
        query: LexicalQuery<CardField>,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        self.do_search_cards_lexical_filtered(query, None)
    }

    pub(crate) fn do_search_cards_lexical_filtered(
        &self,
        query: LexicalQuery<CardField>,
        filter: Option<&dyn Fn(maestria_domain::CardId, maestria_domain::ArtifactId) -> bool>,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "lexical card search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let fields: Vec<_> = query
            .fields
            .iter()
            .map(|s| {
                let field = match s.field {
                    CardField::Title => self.fields.card_title,
                    CardField::Body => self.fields.card_body,
                    CardField::Path => self.fields.card_path,
                    CardField::Filename => self.fields.card_filename,
                    CardField::Symbol => self.fields.card_symbol,
                    CardField::Id => self.fields.card_key,
                };
                (field, s.boost, s.field)
            })
            .collect();

        let searcher = self.reader.searcher();
        let allowed = match filter {
            Some(f) => {
                let keys = self.allowed_card_keys(&searcher, f)?;
                if keys.is_empty() {
                    return Ok(Vec::new());
                }
                Some(keys)
            }
            None => None,
        };
        if query.fields.is_empty() {
            return Err(PortError::InvalidInput {
                message: "lexical card search requires at least one field".to_string(),
            });
        }
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
        let candidate_limit = MAX_LEXICAL_CANDIDATES;
        let top_docs = searcher
            .search(
                &parsed_query,
                &TopDocs::with_limit(candidate_limit).order_by_score(),
            )
            .map_err(to_port_error)?;
        let mut scored = Vec::new();

        let needle = if query.mode == MatchMode::Contains {
            trimmed.to_lowercase()
        } else {
            trimmed.to_string()
        };

        for (_, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            if document.get_first(self.fields.card_id).is_none() {
                continue;
            }
            let card = self.read_lexical_card(&document)?;

            if let Some((raw_score, reason)) = score_card(&card, &query, &needle) {
                scored.push((
                    raw_score,
                    card.artifact_id.value(),
                    card.card_id.value(),
                    card,
                    reason,
                ));
            }
        }

        Ok(page_card_hits(scored, &query))
    }
}
