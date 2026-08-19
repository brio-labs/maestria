use crate::lexical::{IndexedLexicalCard, IndexedLexicalChunk};
use crate::{BoundedSearch, CardHit, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery};
use maestria_domain::{ArtifactId, CardId};

pub trait FullTextIndex: Send + Sync {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError>;
    fn search(&self, query: SearchQuery) -> Result<BoundedSearch<SearchHit>, PortError>;
    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError>;
    fn search_cards(&self, query: SearchQuery) -> Result<BoundedSearch<CardHit>, PortError>;

    /// Delete chunks by their (artifact, chunk) identity, removing every
    /// representation. Adapters without a standalone deletion operation MUST
    /// return an error rather than silently ignoring the request.
    fn delete_chunks(
        &self,
        chunks: &[(maestria_domain::ArtifactId, maestria_domain::ChunkId)],
    ) -> Result<(), PortError> {
        let _ = chunks;
        Err(PortError::InternalContext {
            context: "chunk deletion is unsupported",
            source: "adapter must implement standalone chunk deletion".to_string(),
        })
    }

    /// Remove every document from the index. Adapters without a standalone
    /// clear operation MUST return an error rather than silently ignoring it.
    fn clear(&self) -> Result<(), PortError> {
        Err(PortError::InternalContext {
            context: "full-text clear is unsupported",
            source: "adapter must implement standalone clearing".to_string(),
        })
    }

    /// Execute a search, applying a pre-score filter to candidates.
    /// If an adapter cannot perform pre-filtering natively, it MUST return an error
    /// rather than silently ignoring the filter.
    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(maestria_domain::ChunkId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SearchHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::InternalContext {
            context: "filtered chunk search is unsupported",
            source: "adapter must implement pre-score filtering".to_string(),
        })
    }

    /// Execute a card search, applying a pre-score filter.
    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(CardId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<CardHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::InternalContext {
            context: "filtered card search is unsupported",
            source: "adapter must implement pre-score filtering".to_string(),
        })
    }

    /// Return whether this adapter preserves lexical metadata in its projection.
    fn supports_lexical_metadata(&self) -> bool {
        false
    }

    /// Index chunks with lexical metadata.
    fn index_lexical_chunks(&self, chunks: Vec<IndexedLexicalChunk>) -> Result<(), PortError>;

    /// Index cards with lexical metadata.
    fn index_lexical_cards(&self, cards: Vec<IndexedLexicalCard>) -> Result<(), PortError>;

    /// Index a whole artifact's chunks with its cards as one atomic
    /// projection update.
    ///
    /// The runtime emits one `IndexFullText` effect per pending chunk but
    /// executes them as a per-artifact batch, so ingestion commits once per
    /// artifact instead of once per chunk (chunk commits dominate the
    /// ingestion cost: each flushes and fsyncs segments). The default
    /// implementation preserves the historical call sequence (cards, lexical
    /// cards, chunk, lexical chunk per chunk, with cards attached to the
    /// first chunk), each with its own commit, so adapters without a native
    /// batch path keep identical semantics. Adapters whose writes are costly
    /// per commit SHOULD override this to apply the whole artifact update in
    /// one commit; the operations must stay idempotent (delete-then-add per
    /// key) so retries and recovery re-drives replace rather than duplicate
    /// documents.
    fn index_artifact_chunks(
        &self,
        chunks: Vec<IndexedChunk>,
        cards: Vec<IndexedCard>,
        lexical_chunks: Vec<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        for (index, chunk) in chunks.into_iter().enumerate() {
            let lexical = lexical_chunks
                .iter()
                .find(|candidate| candidate.chunk_id == chunk.chunk_id)
                .cloned();
            if index == 0 && !cards.is_empty() {
                self.index_cards(cards.clone())?;
            }
            if index == 0 && self.supports_lexical_metadata() && !lexical_cards.is_empty() {
                self.index_lexical_cards(lexical_cards.clone())?;
            }
            self.index_chunks(vec![chunk])?;
            if self.supports_lexical_metadata()
                && let Some(lexical_chunk) = lexical
            {
                self.index_lexical_chunks(vec![lexical_chunk])?;
            }
        }
        Ok(())
    }
}
