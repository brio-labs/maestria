use maestria_domain::{Artifact, Card, Chunk, ChunkId, Evidence, EvidenceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchInput {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePack {
    pub query: String,
    pub cards: Vec<SourceGroundedCardHit>,
    pub chunks: Vec<SourceGroundedSearchHit>,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutput {
    pub pack: EvidencePack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGroundedSearchHit {
    pub artifact: Artifact,
    pub chunk: Chunk,
    pub evidence: Evidence,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGroundedCardHit {
    pub artifact: Artifact,
    pub card: Card,
    pub score: u32,
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
