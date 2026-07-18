use maestria_domain::{Artifact, Card, Chunk, ChunkId, Evidence, EvidenceId};

#[path = "evidence_pack.rs"]
mod evidence_pack;
#[path = "evidence_pack_lifecycle.rs"]
mod evidence_pack_lifecycle;
pub use evidence_pack::{
    ClaimCoverageStatus, ClaimEvidenceCoverage, EvidenceFreshness, EvidencePackCompression,
    EvidencePackError, EvidencePackMetadata, EvidencePackReplayKey, EvidencePackReproducibility,
};
pub use evidence_pack_lifecycle::EvidencePack;

#[derive(Debug, Clone, PartialEq)]
pub struct SourceGroundedSearchHit {
    pub artifact: Artifact,
    pub chunk: Chunk,
    pub evidence: Evidence,
    pub score: u32,
    pub lexical_metadata: Option<maestria_ports::LexicalHitMetadata>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceGroundedCardHit {
    pub artifact: Artifact,
    pub card: Card,
    pub score: u32,
    pub lexical_metadata: Option<maestria_ports::LexicalHitMetadata>,
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
