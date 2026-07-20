use maestria_domain::{
    ApprovalId, Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, ClaimId, DomainEventEnvelope,
    Evidence, EvidenceId, MemoryCandidateId,
};

use crate::PortError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFilter {
    pub artifact_id: Option<ArtifactId>,
}

pub trait ArtifactRepository: Send + Sync {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError>;
    fn put(&self, artifact: Artifact) -> Result<(), PortError>;
}

pub trait ChunkRepository: Send + Sync {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError>;
    fn put(&self, chunk: Chunk) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError>;
}

pub trait CardRepository: Send + Sync {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError>;
    fn put(&self, card: Card) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError>;
}

pub trait EvidenceRepository: Send + Sync {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError>;
    /// Insert evidence only if it does not already exist.
    /// Returns `Ok(())` on identical retries; returns `PortError::Conflict`
    /// when a different value already exists for this `EvidenceId`.
    fn put(&self, evidence: Evidence) -> Result<(), PortError>;
    /// Unconditionally store evidence, replacing any existing row.
    fn replace(&self, evidence: Evidence) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError>;
}

pub trait EventLog: Send + Sync {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError>;
    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError>;
}

/// Durable per-namespace ID allocation.
///
/// Each allocation is atomic and persisted so that concurrent or
/// post-restart callers never receive the same ID within a namespace.
pub trait IdAllocator: Send + Sync {
    fn allocate_claim_id(&self) -> Result<ClaimId, PortError>;
    fn allocate_memory_candidate_id(&self) -> Result<MemoryCandidateId, PortError>;
    fn allocate_approval_id(&self) -> Result<ApprovalId, PortError>;
}

pub trait BlobStore: Send + Sync {
    fn put(&self, bytes: Vec<u8>) -> Result<maestria_domain::BlobId, PortError>;
    fn get(&self, id: maestria_domain::BlobId) -> Result<Vec<u8>, PortError>;
}
