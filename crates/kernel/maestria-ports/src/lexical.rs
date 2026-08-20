use maestria_domain::{ArtifactId, CardId, ChunkId};

/// Indexed lexical record for a chunk. The typed lexical *search* family was
/// removed with ADR-0005 (expiry v0.7.0); indexing records remain so
/// `index_lexical_chunks`/`index_lexical_cards` keep feeding the projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedLexicalChunk {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
    pub text: String,
    pub path: Option<String>,
    pub filename: Option<String>,
    pub symbol: Option<String>,
}

/// Indexed lexical record for a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedLexicalCard {
    pub artifact_id: ArtifactId,
    pub card_id: CardId,
    pub title: String,
    pub body: String,
    pub path: Option<String>,
    pub filename: Option<String>,
    pub symbol: Option<String>,
}
