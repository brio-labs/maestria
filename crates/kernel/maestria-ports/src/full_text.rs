use crate::lexical::{
    CardField, ChunkField, IndexedLexicalCard, IndexedLexicalChunk, LexicalCardHit,
    LexicalChunkHit, LexicalQuery,
};
use crate::{CardHit, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery};
use maestria_domain::{ArtifactId, CardId};

pub trait FullTextIndex: Send + Sync {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError>;
    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError>;
    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError>;
    fn search_cards(&self, query: SearchQuery) -> Result<Vec<CardHit>, PortError>;

    /// Execute a search, applying a pre-score filter to candidates.
    /// If an adapter cannot perform pre-filtering natively, it MUST return an error
    /// rather than silently ignoring the filter.
    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(maestria_domain::ChunkId, ArtifactId) -> bool,
    ) -> Result<Vec<SearchHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::Internal {
            message: "search_filtered not supported by this index".into(),
        })
    }

    /// Execute a card search, applying a pre-score filter.
    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(CardId, ArtifactId) -> bool,
    ) -> Result<Vec<CardHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::Internal {
            message: "search_cards_filtered not supported by this index".into(),
        })
    }

    /// Return whether this adapter preserves lexical metadata in its projection.
    fn supports_lexical_metadata(&self) -> bool {
        false
    }

    /// Fallback to legacy index_chunks, dropping lexical metadata.
    fn index_lexical_chunks(&self, chunks: Vec<IndexedLexicalChunk>) -> Result<(), PortError> {
        self.index_chunks(
            chunks
                .into_iter()
                .map(|c| IndexedChunk {
                    artifact_id: c.artifact_id,
                    chunk_id: c.chunk_id,
                    text: c.text,
                })
                .collect(),
        )
    }

    /// Fallback to legacy index_cards, dropping lexical metadata.
    fn index_lexical_cards(&self, cards: Vec<IndexedLexicalCard>) -> Result<(), PortError> {
        self.index_cards(
            cards
                .into_iter()
                .map(|c| IndexedCard {
                    artifact_id: c.artifact_id,
                    card_id: c.card_id,
                    title: c.title,
                    body: c.body,
                })
                .collect(),
        )
    }

    /// Execute a typed lexical search for chunks.
    fn search_lexical(
        &self,
        query: LexicalQuery<ChunkField>,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        let _ = query;
        Err(PortError::Internal {
            message: "search_lexical not supported by this index".into(),
        })
    }

    /// Execute a typed lexical search for cards.
    fn search_cards_lexical(
        &self,
        query: LexicalQuery<CardField>,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        let _ = query;
        Err(PortError::Internal {
            message: "search_cards_lexical not supported by this index".into(),
        })
    }

    /// Execute a typed lexical search for chunks, applying a pre-score filter to candidates.
    /// This method is REQUIRED for governed retrieval to enforce ACL/scope boundaries securely.
    fn search_lexical_filtered(
        &self,
        query: LexicalQuery<ChunkField>,
        filter: &dyn Fn(maestria_domain::ChunkId, ArtifactId) -> bool,
    ) -> Result<Vec<LexicalChunkHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::Internal {
            message: "search_lexical_filtered not supported by this index".into(),
        })
    }

    /// Execute a typed lexical search for cards, applying a pre-score filter to candidates.
    /// This method is REQUIRED for governed retrieval to enforce ACL/scope boundaries securely.
    fn search_cards_lexical_filtered(
        &self,
        query: LexicalQuery<CardField>,
        filter: &dyn Fn(CardId, ArtifactId) -> bool,
    ) -> Result<Vec<LexicalCardHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::Internal {
            message: "search_cards_lexical_filtered not supported by this index".into(),
        })
    }
}
