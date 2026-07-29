use crate::entities::{
    Artifact, Card, Chunk, Claim, Evidence, Memory, MemoryCandidate, PendingArtifact, Relation,
    Task, ValidationReportRecord,
};
use crate::events::DomainEventEnvelope;
use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, CardId, ChunkId, ClaimId, EvidenceId, HarnessRunId,
    MemoryCandidateId, MemoryId, RelationId, StructureNodeId, TaskId, ValidationReportId,
};
use crate::inputs::ParserStarted;
use crate::search::{ContentHash, StructureNode};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KernelState {
    pub artifacts: BTreeMap<ArtifactId, Artifact>,
    pub artifact_versions: BTreeMap<ArtifactId, ArtifactVersionId>,
    pub artifact_content_hashes: BTreeMap<ArtifactId, ContentHash>,
    pub document_trees: BTreeMap<ArtifactId, (StructureNodeId, Vec<StructureNode>)>,
    pub pending_artifacts: BTreeMap<ArtifactId, PendingArtifact>,
    pub pending_parsers: BTreeMap<ArtifactId, ParserStarted>,
    pub pending_ocr: BTreeMap<crate::ocr::OcrRequestId, crate::ocr::OcrIntent>,
    pub ocr_intents: BTreeMap<crate::ocr::OcrRequestId, crate::ocr::OcrIntent>,
    pub ocr_results: BTreeMap<crate::ocr::OcrRequestId, crate::ocr::OcrCompletion>,
    pub ocr_failures: BTreeMap<crate::ocr::OcrRequestId, String>,
    pub chunk_nodes: BTreeMap<ChunkId, StructureNodeId>,
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
    pub model_agent_requests: BTreeMap<HarnessRunId, crate::model_agent::ModelAgentProposalRequest>,
    pub model_agent_results: BTreeMap<HarnessRunId, crate::model_agent::ModelAgentProposalResult>,
    pub pending_full_text: BTreeSet<ChunkId>,
    pub parsed_artifact_ids: BTreeSet<ArtifactId>,
    pub stale_sources: BTreeSet<String>,
    pub index_generations: crate::generations::IndexGenerationRegistry,
    pub event_log: Vec<DomainEventEnvelope>,
}
