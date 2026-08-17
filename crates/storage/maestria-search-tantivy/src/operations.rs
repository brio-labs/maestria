use crate::tantivy_index::TantivyFullTextIndex;
use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{
    BoundedSearch, CardField, CardHit, ChunkField, FullTextIndex, IndexedCard, IndexedChunk,
    IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit, LexicalChunkHit, LexicalQuery,
    PortError, SearchHit, SearchQuery,
};

impl FullTextIndex for TantivyFullTextIndex {
    fn supports_lexical_metadata(&self) -> bool {
        true
    }

    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        self.index_chunks_impl(chunks)
    }

    fn delete_chunks(
        &self,
        chunks: &[(maestria_domain::ArtifactId, maestria_domain::ChunkId)],
    ) -> Result<(), PortError> {
        self.delete_chunks_impl(chunks)
    }

    fn clear(&self) -> Result<(), PortError> {
        self.clear_impl()
    }

    fn search(&self, query: SearchQuery) -> Result<BoundedSearch<SearchHit>, PortError> {
        self.search_chunks_impl(query)
    }

    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SearchHit>, PortError> {
        self.search_chunks_filtered_impl(query, filter)
    }

    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        self.index_cards_impl(cards)
    }

    fn index_artifact_chunk(
        &self,
        chunk: IndexedChunk,
        cards: Vec<IndexedCard>,
        lexical_chunk: Option<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.index_artifact_chunk_impl(chunk, cards, lexical_chunk, lexical_cards)
    }

    fn index_artifact_chunks(
        &self,
        chunks: Vec<IndexedChunk>,
        cards: Vec<IndexedCard>,
        lexical_chunks: Vec<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.index_artifact_chunks_impl(chunks, cards, lexical_chunks, lexical_cards)
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
    ) -> Result<BoundedSearch<LexicalChunkHit>, PortError> {
        self.do_search_lexical(query)
    }

    fn search_lexical_filtered(
        &self,
        query: LexicalQuery<ChunkField>,
        filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<LexicalChunkHit>, PortError> {
        self.do_search_lexical_filtered(query, Some(filter))
    }

    fn search_cards_lexical(
        &self,
        query: LexicalQuery<CardField>,
    ) -> Result<BoundedSearch<LexicalCardHit>, PortError> {
        self.do_search_cards_lexical(query)
    }

    fn search_cards_lexical_filtered(
        &self,
        query: LexicalQuery<CardField>,
        filter: &dyn Fn(CardId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<LexicalCardHit>, PortError> {
        self.do_search_cards_lexical_filtered(query, Some(filter))
    }

    fn search_cards(&self, query: SearchQuery) -> Result<BoundedSearch<CardHit>, PortError> {
        self.search_cards_impl(query)
    }

    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(CardId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<CardHit>, PortError> {
        self.search_cards_filtered_impl(query, filter)
    }
}

use crate::error::to_port_error;
use crate::keys::{card_key, chunk_key};
use tantivy::Term;

impl TantivyFullTextIndex {
    /// Index one artifact chunk with its cards and lexical metadata in a
    /// single commit. Delegates to the batched whole-artifact write; see
    /// [`Self::index_artifact_chunks_impl`].
    pub(crate) fn index_artifact_chunk_impl(
        &self,
        chunk: IndexedChunk,
        cards: Vec<IndexedCard>,
        lexical_chunk: Option<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.index_artifact_chunks_impl(
            vec![chunk],
            cards,
            lexical_chunk.into_iter().collect(),
            lexical_cards,
        )
    }

    /// Index a whole artifact's chunks with its cards and lexical metadata
    /// in a single commit.
    ///
    /// This is the hot path for artifact ingestion: the runtime full-text
    /// effect batches every pending chunk of one artifact, so a home-scale
    /// corpus costs one commit per artifact instead of one per chunk.
    /// Tantivy commits flush and fsync segments, so the batch replaces the
    /// historical per-write commits (cards, lexical cards, chunk, lexical
    /// chunk). The delete-then-add pattern is preserved per key, so retries
    /// and recovery re-drives stay idempotent; the whole artifact update
    /// becomes visible atomically.
    pub(crate) fn index_artifact_chunks_impl(
        &self,
        chunks: Vec<IndexedChunk>,
        cards: Vec<IndexedCard>,
        lexical_chunks: Vec<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        let mut writer_guard = self.writer.lock().map_err(|_| PortError::InternalContext {
            context: "Tantivy writer lock poisoned",
            source: "Tantivy writer mutex is poisoned".to_string(),
        })?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "index artifact chunks requires a writable full-text index",
                source: "full-text index is read-only".to_string(),
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
        for card in lexical_cards {
            writer.delete_term(Term::from_field_text(
                self.fields.card_key,
                &card_key(card.artifact_id, card.card_id),
            ));
            writer
                .add_document(self.lexical_card_document(&card))
                .map_err(to_port_error)?;
        }
        for chunk in &chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(chunk.artifact_id, chunk.chunk_id),
            ));
            writer
                .add_document(self.chunk_document(chunk))
                .map_err(to_port_error)?;
        }
        for lexical_chunk in &lexical_chunks {
            writer.delete_term(Term::from_field_text(
                self.fields.key,
                &chunk_key(lexical_chunk.artifact_id, lexical_chunk.chunk_id),
            ));
            writer
                .add_document(self.lexical_chunk_document(lexical_chunk))
                .map_err(to_port_error)?;
        }
        writer.commit().map_err(to_port_error)?;
        self.reader.reload().map_err(to_port_error)
    }
}
