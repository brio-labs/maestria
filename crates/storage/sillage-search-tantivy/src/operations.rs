use crate::tantivy_index::TantivyFullTextIndex;
use sillage_domain::{ArtifactId, CardId, ChunkId};
use sillage_ports::{
    BoundedSearch, CardHit, FullTextIndex, IndexedCard, IndexedChunk, IndexedLexicalCard,
    IndexedLexicalChunk, PortError, SearchHit, SearchQuery,
};

impl FullTextIndex for TantivyFullTextIndex {
    fn commit_and_reload(&self) -> Result<(), PortError> {
        TantivyFullTextIndex::commit_and_reload(self)
    }

    fn supports_lexical_metadata(&self) -> bool {
        true
    }

    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        self.index_chunks_impl(chunks)
    }

    fn delete_chunks(
        &self,
        chunks: &[(sillage_domain::ArtifactId, sillage_domain::ChunkId)],
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

use crate::memory_document::BudgetedDocument;
use std::mem::size_of;

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
                let count = chunks
                    .len()
                    .checked_add(cards.len())
                    .and_then(|count| count.checked_add(lexical_chunks.len()))
                    .and_then(|count| count.checked_add(lexical_cards.len()))
                    .ok_or_else(|| self.accounting_overflow())?;
                let bytes = count
                    .checked_mul(size_of::<BudgetedDocument>())
                    .and_then(|bytes| u64::try_from(bytes).ok())
                    .ok_or_else(|| self.accounting_overflow())?;
                let _batch_reservation = self.reserve_document_memory(bytes)?;
                let mut documents = Vec::with_capacity(count);
                for card in cards {
                    documents.push(self.card_document(card)?);
                }
                for card in lexical_cards {
                    documents.push(self.lexical_card_document(card)?);
                }
                for chunk in chunks {
                    documents.push(self.chunk_document(chunk)?);
                }
                for chunk in lexical_chunks {
                    documents.push(self.lexical_chunk_document(chunk)?);
                }
                self.reserve_writer_batch(&documents, std::iter::empty())?;
                for mut document in documents {
                    self.mark_dirty();
                    writer.delete_term(document.take_delete_term());
                    writer
                        .add_document(document)
                        .map_err(crate::error::to_port_error)?;
                }
                Ok(())
            },
        )
    }
}
