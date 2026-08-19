use crate::GrantTokenDigest;
use crate::entities::{
    Artifact, Card, Chunk, Claim, Evidence, Memory, MemoryCandidate, PendingArtifact,
    RealmReadGrant, Relation, Task, ValidationReportRecord,
};
use crate::events::DomainEventEnvelope;
use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, CardId, ChunkId, ClaimId, EvidenceId, HarnessRunId,
    LogicalTick, MemoryCandidateId, MemoryId, NotebookDraftId, NotebookId, RelationId,
    StructureNodeId, TaskId, ValidationReportId,
};
use crate::inputs::ParserStarted;
use crate::notebook::{Notebook, NotebookDraft, SourceIdentityKey};
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
    pub notebooks: BTreeMap<NotebookId, Notebook>,
    pub notebook_drafts: BTreeMap<NotebookDraftId, NotebookDraft>,
    pub active_sources: BTreeMap<SourceIdentityKey, ArtifactId>,
    pub index_generations: crate::generations::IndexGenerationRegistry,
    /// Rebuildable provider grant current state. The event log is authoritative.
    pub realm_read_grants: BTreeMap<GrantTokenDigest, RealmReadGrant>,
    /// Cached latest observed clock tick. Written by `apply_tick_observed`
    /// (replay) and `process_clock_tick` (live), both from the same event
    /// content, so the cache is deterministic. `None` until the first tick.
    pub current_tick: Option<LogicalTick>,
    pub event_log: Vec<DomainEventEnvelope>,
}
