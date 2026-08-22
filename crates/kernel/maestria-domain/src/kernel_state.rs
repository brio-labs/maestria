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
use std::sync::Arc;

/// Copy-on-write shared map/set used by [`KernelState`] so whole-state
/// snapshots clone pointer-size handles instead of deep-cloning every
/// entity; handlers mutate through [`Arc::make_mut`], cloning only the one
/// collection an input actually touches.
type CowMap<K, V> = Arc<BTreeMap<K, V>>;
type CowSet<V> = Arc<BTreeSet<V>>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KernelState {
    pub artifacts: CowMap<ArtifactId, Artifact>,
    pub artifact_versions: CowMap<ArtifactId, ArtifactVersionId>,
    pub artifact_content_hashes: CowMap<ArtifactId, ContentHash>,
    pub document_trees: CowMap<ArtifactId, (StructureNodeId, Vec<StructureNode>)>,
    pub pending_artifacts: CowMap<ArtifactId, PendingArtifact>,
    pub pending_parsers: CowMap<ArtifactId, ParserStarted>,
    pub pending_ocr: CowMap<crate::ocr::OcrRequestId, crate::ocr::OcrIntent>,
    pub ocr_intents: CowMap<crate::ocr::OcrRequestId, crate::ocr::OcrIntent>,
    pub ocr_results: CowMap<crate::ocr::OcrRequestId, crate::ocr::OcrCompletion>,
    pub ocr_failures: CowMap<crate::ocr::OcrRequestId, String>,
    pub chunks: CowMap<ChunkId, Chunk>,
    pub cards: CowMap<CardId, Card>,
    pub evidences: CowMap<EvidenceId, Evidence>,
    pub claims: CowMap<ClaimId, Claim>,
    pub relations: CowMap<RelationId, Relation>,
    pub memory_candidates: CowMap<MemoryCandidateId, MemoryCandidate>,
    pub memories: CowMap<MemoryId, Memory>,
    pub tasks: CowMap<TaskId, Task>,
    pub validation_reports: CowMap<ValidationReportId, ValidationReportRecord>,
    pub resolved_approvals: CowSet<ApprovalId>,
    pub model_agent_requests: CowMap<HarnessRunId, crate::model_agent::ModelAgentProposalRequest>,
    pub model_agent_results: CowMap<HarnessRunId, crate::model_agent::ModelAgentProposalResult>,
    pub pending_full_text: CowSet<ChunkId>,
    pub parsed_artifact_ids: CowSet<ArtifactId>,
    pub stale_sources: CowSet<String>,
    pub notebooks: CowMap<NotebookId, Notebook>,
    pub notebook_drafts: CowMap<NotebookDraftId, NotebookDraft>,
    pub active_sources: CowMap<SourceIdentityKey, ArtifactId>,
    pub index_generations: crate::generations::IndexGenerationRegistry,
    /// Rebuildable provider grant current state. The event log is authoritative.
    pub realm_read_grants: CowMap<GrantTokenDigest, RealmReadGrant>,
    /// Cached latest observed clock tick. Written by `apply_tick_observed`
    /// (replay) and `process_clock_tick` (live), both from the same event
    /// content, so the cache is deterministic. `None` until the first tick.
    pub current_tick: Option<LogicalTick>,
    /// Append-shared log: cloning the state bumps one element refcount per
    /// entry instead of deep-cloning every envelope.
    pub event_log: Vec<Arc<DomainEventEnvelope>>,
}

impl KernelState {
    /// Owned copy of the durable log for cold replay paths (tests, startup
    /// diagnostics). Clones per envelope by design; hot paths iterate
    /// `&self.event_log` and deref the shared handles instead.
    pub fn event_log_owned(&self) -> Vec<DomainEventEnvelope> {
        self.event_log
            .iter()
            .map(|envelope| envelope.as_ref().clone())
            .collect()
    }
}
