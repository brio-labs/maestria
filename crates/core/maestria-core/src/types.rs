use maestria_domain::{Artifact, Chunk, ChunkId, Evidence, EvidenceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchInput {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutput {
    pub hits: Vec<SourceGroundedSearchHit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGroundedSearchHit {
    pub artifact: Artifact,
    pub chunk: Chunk,
    pub evidence: Evidence,
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
