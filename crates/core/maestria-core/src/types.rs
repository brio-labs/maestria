use maestria_domain::{Artifact, Card, Chunk, ChunkId, Evidence, EvidenceId};
use maestria_ports::LexicalHitMetadata;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchInput {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidencePack {
    pub query: String,
    pub cards: Vec<SourceGroundedCardHit>,
    pub chunks: Vec<SourceGroundedSearchHit>,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalMode {
    LexicalOnly,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchOutput {
    pub pack: EvidencePack,
    pub mode: RetrievalMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceGroundedSearchHit {
    pub artifact: Artifact,
    pub chunk: Chunk,
    pub evidence: Evidence,
    pub score: u32,
    pub lexical_metadata: Option<LexicalHitMetadata>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceGroundedCardHit {
    pub artifact: Artifact,
    pub card: Card,
    pub score: u32,
    pub lexical_metadata: Option<LexicalHitMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenEvidenceInput {
    pub evidence_id: EvidenceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenChunkEvidenceInput {
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenEvidenceOutput {
    pub artifact: Artifact,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphConfig {
    pub max_depth: usize,
    pub max_results: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_results: 10,
        }
    }
}
