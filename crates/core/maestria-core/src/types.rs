use maestria_domain::{Artifact, ArtifactVersionId, Card, Chunk, ChunkId, Evidence, EvidenceId};

#[derive(Debug, Clone, PartialEq)]
pub struct SourceGroundedSearchHit {
    pub artifact: Artifact,
    pub artifact_version_id: ArtifactVersionId,
    pub chunk: Chunk,
    pub evidence: Evidence,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq)]
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
