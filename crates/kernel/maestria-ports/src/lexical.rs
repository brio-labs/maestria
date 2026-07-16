use maestria_domain::{ArtifactId, CardId, ChunkId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedLexicalChunk {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
    pub text: String,
    pub path: Option<String>,
    pub filename: Option<String>,
    pub symbol: Option<String>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchMode {
    Contains,
    Exact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkField {
    Text,
    Path,
    Filename,
    Symbol,
    Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardField {
    Title,
    Body,
    Path,
    Filename,
    Symbol,
    Id,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldSelector<F> {
    pub field: F,
    pub boost: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexicalQuery<F> {
    pub q: String,
    pub limit: usize,
    pub offset: usize,
    pub mode: MatchMode,
    pub fields: Vec<FieldSelector<F>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrieverIdentity {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitReason {
    ExactMatch { field: String },
    FieldMatch { field: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexicalHitMetadata {
    pub retriever: RetrieverIdentity,
    pub raw_score: f32,
    pub raw_rank: u32,
    pub reason: HitReason,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexicalChunkHit {
    pub chunk: IndexedLexicalChunk,
    pub metadata: LexicalHitMetadata,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexicalCardHit {
    pub card: IndexedLexicalCard,
    pub metadata: LexicalHitMetadata,
}
