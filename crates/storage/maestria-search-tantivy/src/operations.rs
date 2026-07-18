use super::lexical_helpers::MAX_LEXICAL_CANDIDATES;
use super::search_helpers::collect_tie_complete;
use super::{
    TantivyFullTextIndex, card_key, chunk_key, descending_score, score_to_u32, to_port_error,
};
use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{
    CardField, CardHit, ChunkField, FullTextIndex, IndexedCard, IndexedChunk, IndexedLexicalCard,
    IndexedLexicalChunk, LexicalCardHit, LexicalChunkHit, LexicalQuery, PortError, SearchHit,
    SearchQuery,
};
use tantivy::{
    TantivyDocument, Term,
    collector::DocSetCollector,
    query::{AllQuery, BooleanQuery, QueryParser, TermSetQuery},
};

impl FullTextIndex for TantivyFullTextIndex {
    fn supports_lexical_metadata(&self) -> bool {
        true
    }
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::Internal {
            message: "tantivy writer lock poisoned".to_string(),
        })?;
        let writer = writer_guard.as_mut().ok_or_else(|| PortError::Downstream {
            message: "full-text index is read-only".to_string(),
        })?;
        for chunk in chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(chunk.artifact_id, chunk.chunk_id),
            ));
            writer
                .add_document(self.chunk_document(&chunk))
                .map_err(to_port_error)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.fields.text]);
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInput {
                    message: format!("invalid search query: {error}"),
                })?;
        let top_docs = collect_tie_complete(&searcher, &parsed_query, query.offset, query.limit)?;
        let mut scored = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let chunk = self.read_chunk(&document)?;
            scored.push((
                score,
                chunk.artifact_id.value(),
                chunk.chunk_id.value(),
                chunk,
            ));
        }
        scored.sort_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok(scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, chunk)| SearchHit {
                chunk,
                score: score_to_u32(score),
            })
            .collect())
    }

    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(ChunkId, ArtifactId) -> bool,
    ) -> Result<Vec<SearchHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let allowed = self.allowed_chunk_keys(&searcher, filter)?;
        if allowed.is_empty() {
            return Ok(Vec::new());
        }
        let parser = QueryParser::for_index(&self.index, vec![self.fields.text]);
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInput {
                    message: format!("invalid search query: {error}"),
                })?;
        let scoped_query = BooleanQuery::intersection(vec![
            parsed_query,
            Box::new(TermSetQuery::new(
                allowed
                    .into_iter()
                    .map(|key| Term::from_field_text(self.fields.key, &key)),
            )),
        ]);
        let top_docs = collect_tie_complete(&searcher, &scoped_query, query.offset, query.limit)?;
        let mut scored = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let chunk = self.read_chunk(&document)?;
            scored.push((
                score,
                chunk.artifact_id.value(),
                chunk.chunk_id.value(),
                chunk,
            ));
        }
        scored.sort_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok(scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, chunk)| SearchHit {
                chunk,
                score: score_to_u32(score),
            })
            .collect())
    }

    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::Internal {
            message: "tantivy writer lock poisoned".to_string(),
        })?;
        let writer = writer_guard.as_mut().ok_or_else(|| PortError::Downstream {
            message: "full-text index is read-only".to_string(),
        })?;
        for card in cards {
            writer.delete_term(Term::from_field_text(
                self.fields.card_key,
                &card_key(card.artifact_id, card.card_id),
            ));
            writer
                .add_document(self.card_document(&card))
                .map_err(to_port_error)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }

    fn index_lexical_chunks(&self, chunks: Vec<IndexedLexicalChunk>) -> Result<(), PortError> {
        self.do_index_lexical_chunks(chunks)
    }

    fn index_lexical_cards(&self, cards: Vec<IndexedLexicalCard>) -> Result<(), PortError> {
        self.do_index_lexical_cards(cards)
    }

    fn search_lexical(
        &self,
        query: LexicalQuery<ChunkField>,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        self.do_search_lexical(query)
    }

    fn search_lexical_filtered(
        &self,
        query: LexicalQuery<ChunkField>,
        filter: &dyn Fn(ChunkId, ArtifactId) -> bool,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        self.do_search_lexical_filtered(query, Some(filter))
    }

    fn search_cards_lexical(
        &self,
        query: LexicalQuery<CardField>,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        self.do_search_cards_lexical(query)
    }

    fn search_cards_lexical_filtered(
        &self,
        query: LexicalQuery<CardField>,
        filter: &dyn Fn(CardId, ArtifactId) -> bool,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        self.do_search_cards_lexical_filtered(query, Some(filter))
    }

    fn search_cards(&self, query: SearchQuery) -> Result<Vec<CardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.card_title, self.fields.card_body],
        );
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInput {
                    message: format!("invalid search query: {error}"),
                })?;
        let top_docs = collect_tie_complete(&searcher, &parsed_query, query.offset, query.limit)?;
        let mut scored: Vec<(f32, u64, u64, IndexedCard)> = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let card = self.read_card(&document)?;
            scored.push((score, card.artifact_id.value(), card.card_id.value(), card));
        }
        scored.sort_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok(scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, card)| CardHit {
                card,
                score: score_to_u32(score),
            })
            .collect())
    }

    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(CardId, ArtifactId) -> bool,
    ) -> Result<Vec<CardHit>, PortError> {
        let trimmed = query.q.trim();
        if trimmed.is_empty() {
            return Err(PortError::InvalidInput {
                message: "search query must not be empty".to_string(),
            });
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let allowed = self.allowed_card_keys(&searcher, filter)?;
        if allowed.is_empty() {
            return Ok(Vec::new());
        }
        let parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.card_title, self.fields.card_body],
        );
        let parsed_query =
            parser
                .parse_query(trimmed)
                .map_err(|error| PortError::InvalidInput {
                    message: format!("invalid search query: {error}"),
                })?;
        let scoped_query = BooleanQuery::intersection(vec![
            parsed_query,
            Box::new(TermSetQuery::new(
                allowed
                    .into_iter()
                    .map(|key| Term::from_field_text(self.fields.card_key, &key)),
            )),
        ]);
        let top_docs = collect_tie_complete(&searcher, &scoped_query, query.offset, query.limit)?;
        let mut scored = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            let card = self.read_card(&document)?;
            scored.push((score, card.artifact_id.value(), card.card_id.value(), card));
        }
        scored.sort_by(|a, b| {
            descending_score(a.0, b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        Ok(scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(score, _, _, card)| CardHit {
                card,
                score: score_to_u32(score),
            })
            .collect())
    }
}

impl TantivyFullTextIndex {
    pub(crate) fn allowed_chunk_keys(
        &self,
        searcher: &tantivy::Searcher,
        filter: &dyn Fn(ChunkId, ArtifactId) -> bool,
    ) -> Result<Vec<String>, PortError> {
        let addresses = searcher
            .search(&AllQuery, &DocSetCollector)
            .map_err(to_port_error)?;
        let mut allowed = std::collections::BTreeSet::new();
        for address in addresses {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            if document.get_first(self.fields.chunk_id).is_none() {
                continue;
            }
            let chunk = self.read_chunk(&document)?;
            if filter(chunk.chunk_id, chunk.artifact_id) {
                if allowed.len() >= MAX_LEXICAL_CANDIDATES {
                    return Err(PortError::Internal {
                        message: "filtered lexical candidate set exceeds bounded limit".to_string(),
                    });
                }
                allowed.insert(chunk_key(chunk.artifact_id, chunk.chunk_id));
            }
        }
        Ok(allowed.into_iter().collect())
    }

    pub(crate) fn allowed_card_keys(
        &self,
        searcher: &tantivy::Searcher,
        filter: &dyn Fn(CardId, ArtifactId) -> bool,
    ) -> Result<Vec<String>, PortError> {
        let addresses = searcher
            .search(&AllQuery, &DocSetCollector)
            .map_err(to_port_error)?;
        let mut allowed = std::collections::BTreeSet::new();
        for address in addresses {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(to_port_error)?;
            if document.get_first(self.fields.card_id).is_none() {
                continue;
            }
            let card = self.read_card(&document)?;
            if filter(card.card_id, card.artifact_id) {
                if allowed.len() >= MAX_LEXICAL_CANDIDATES {
                    return Err(PortError::Internal {
                        message: "filtered lexical candidate set exceeds bounded limit".to_string(),
                    });
                }
                allowed.insert(card_key(card.artifact_id, card.card_id));
            }
        }
        Ok(allowed.into_iter().collect())
    }
}
