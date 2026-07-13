use crate::entities::{
    Artifact, Card, Chunk, Claim, Evidence, Memory, MemoryCandidate, PendingArtifact, Relation,
    Task, ValidationReportRecord,
};
use crate::events::DomainEventEnvelope;
use crate::ids::{
    ApprovalId, ArtifactId, CardId, ChunkId, ClaimId, EvidenceId, MemoryCandidateId, MemoryId,
    RelationId, TaskId, ValidationReportId,
};
use crate::inputs::ParserStarted;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KernelState {
    pub artifacts: BTreeMap<ArtifactId, Artifact>,
    pub pending_artifacts: BTreeMap<ArtifactId, PendingArtifact>,
    pub pending_parsers: BTreeMap<ArtifactId, ParserStarted>,
    pub chunks: BTreeMap<ChunkId, Chunk>,
    pub cards: BTreeMap<CardId, Card>,
    pub evidences: BTreeMap<EvidenceId, Evidence>,
    pub claims: BTreeMap<ClaimId, Claim>,
    pub relations: BTreeMap<RelationId, Relation>,
    pub memory_candidates: BTreeMap<MemoryCandidateId, MemoryCandidate>,
    pub memories: BTreeMap<MemoryId, Memory>,
    pub tasks: BTreeMap<TaskId, Task>,
    pub validation_reports: BTreeMap<ValidationReportId, ValidationReportRecord>,
    pub resolved_approvals: BTreeSet<ApprovalId>,
    pub pending_full_text: BTreeSet<ChunkId>,
    pub parsed_artifact_ids: BTreeSet<ArtifactId>,
    pub event_log: Vec<DomainEventEnvelope>,
}
