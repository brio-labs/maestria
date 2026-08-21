use crate::tantivy_index::TantivyFullTextIndex;
use maestria_domain::{ArtifactId, CardId, ChunkId};
use maestria_ports::{
    BoundedSearch, CardHit, FullTextIndex, IndexedCard, IndexedChunk, IndexedLexicalCard,
    IndexedLexicalChunk, PortError, SearchHit, SearchQuery,
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
    pub(crate) fn index_artifact_chunks_impl(
        &self,
        chunks: Vec<IndexedChunk>,
        cards: Vec<IndexedCard>,
        lexical_chunks: Vec<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.with_writer(
            "index artifact chunks requires a writable full-text index",
            |writer| {
                for card in cards {
                    writer.delete_term(Term::from_field_text(
                        self.fields.card_key,
                        &card_key(card.artifact_id, card.card_id),
                    ));
                    writer
                        .add_document(self.card_document(card))
                        .map_err(to_port_error)?;
                }
                for card in lexical_cards {
                    writer.delete_term(Term::from_field_text(
                        self.fields.card_key,
                        &card_key(card.artifact_id, card.card_id),
                    ));
                    writer
                        .add_document(self.lexical_card_document(card))
                        .map_err(to_port_error)?;
                }
                for chunk in chunks {
                    writer.delete_term(Term::from_field_text(
                        self.fields.key,
                        &chunk_key(chunk.artifact_id, chunk.chunk_id),
                    ));
                    writer
                        .add_document(self.chunk_document(chunk))
                        .map_err(to_port_error)?;
                }
                for lexical_chunk in lexical_chunks {
                    writer.delete_term(Term::from_field_text(
                        self.fields.key,
                        &chunk_key(lexical_chunk.artifact_id, lexical_chunk.chunk_id),
                    ));
                    writer
                        .add_document(self.lexical_chunk_document(lexical_chunk))
                        .map_err(to_port_error)?;
                }
                Ok(())
            },
        )
    }
}
