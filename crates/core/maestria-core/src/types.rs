use maestria_domain::{Artifact, Chunk, Evidence, EvidenceId, LogicalTick};
use maestria_domain::{ArtifactId, ChunkId};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestFileInput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub observed_at: LogicalTick,
    pub artifact_id: Option<ArtifactId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestFileOutput {
    pub artifact: Artifact,
    pub chunks: Vec<Chunk>,
    pub evidence: Vec<Evidence>,
    pub blob_id: maestria_domain::BlobId,
    pub content_hash: String,
    pub unchanged: bool,
}

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
