use crate::ids::{ArtifactId, CardId, ClaimId, EvidenceId, MemoryId, RelationId, TaskId};
use crate::security::SecurityMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub id: RelationId,
    pub source: RelationEndpoint,
    pub kind: RelationKind,
    pub target: RelationEndpoint,
    pub evidence_id: Option<EvidenceId>,
    pub confidence_milli: u16,
    pub security: SecurityMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelationEndpoint {
    Artifact(ArtifactId),
    Claim(ClaimId),
    Task(TaskId),
    Memory(MemoryId),
    Card(CardId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Contains,
    Defines,
    Supports,
    Contradicts,
    UsedEvidence,
    BasedOn,
    DerivedFrom,
    AppliesTo,
    RelatedTo,
}
