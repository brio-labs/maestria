use std::path::PathBuf;

use maestria_domain::{ArtifactId, CardId, ChunkId, SearchExecution, SearchExecutionBudget};

#[derive(Debug, Clone, PartialEq)]
pub struct BoundedSearch<T> {
    pub hits: Vec<T>,
    pub execution: SearchExecution,
}

impl<T> BoundedSearch<T> {
    pub const fn new(hits: Vec<T>, execution: SearchExecution) -> Self {
        Self { hits, execution }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedChunk {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedCard {
    pub artifact_id: ArtifactId,
    pub card_id: CardId,
    pub title: String,
    pub body: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    pub q: String,
    pub limit: usize,
    /// Number of matching documents to skip before applying `limit`.
    pub offset: usize,
    pub execution_budget: SearchExecutionBudget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub chunk: IndexedChunk,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardHit {
    pub card: IndexedCard,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub size: usize,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHandle {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}
