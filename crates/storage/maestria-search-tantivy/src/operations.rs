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
