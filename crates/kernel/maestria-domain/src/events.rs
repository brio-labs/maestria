use crate::entities::{ClaimStatus, RelationEndpoint, RelationKind, TaskPriority};
use crate::evidence_source::EvidenceKind;
use crate::ids::StructureNodeId;
use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, BlobId, CardId, ChunkId, ClaimId, EventId,
    EvidenceId, IndexGenerationId, LogicalTick, MemoryCandidateId, MemoryId, RelationId,
    SequenceNumber, TaskId, ValidationReportId,
};
use crate::search::{ContentHash, StructureNode};
use crate::security::SecurityMetadata;
use crate::task_status::TaskStatus;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainEventEnvelope {
    pub id: EventId,
    pub sequence: SequenceNumber,
    pub event: DomainEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    ArtifactRegistered {
        artifact_id: ArtifactId,
        title: String,
        security: SecurityMetadata,
    },
    ChunkRegistered {
        chunk_id: ChunkId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        representations: Vec<crate::provenance::ParsedRepresentation>,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: CardId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        title: String,
        body: String,
        security: SecurityMetadata,
    },
    ClaimCreated {
        claim_id: ClaimId,
        artifact_id: ArtifactId,
        text: String,
        evidence_ids: Vec<EvidenceId>,
        security: SecurityMetadata,
    },
    EvidenceRecorded {
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
        excerpt: String,
        observed_at: LogicalTick,
        security: SecurityMetadata,
    },
    TaskOpened {
        task_id: TaskId,
        title: String,
        priority: TaskPriority,
        artifact_id: Option<ArtifactId>,
    },
    TaskStatusChanged {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    },
    /// The completed status carries its validation report (R56).
    TaskCompletionRecorded {
        task_id: TaskId,
        status: TaskStatus,
    },
    TaskEvidenceLinked {
        task_id: TaskId,
        evidence_id: EvidenceId,
    },
    ClaimValidationUpdated {
        claim_id: ClaimId,
        status: ClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: ClaimId,
        evidence_id: EvidenceId,
    },
    RelationCreated {
        relation_id: RelationId,
        source: RelationEndpoint,
        kind: RelationKind,
        target: RelationEndpoint,
        evidence_id: Option<EvidenceId>,
        confidence_milli: u16,
        security: SecurityMetadata,
    },
    MemoryCandidateCreated {
        candidate_id: MemoryCandidateId,
        claim_id: ClaimId,
        evidence_ids: BTreeSet<EvidenceId>,
        confidence_milli: u16,
        security: SecurityMetadata,
    },
    UserIntentObserved {
        task_id: TaskId,
        title: String,
    },
    ArtifactParsed {
        artifact_id: ArtifactId,
        status: crate::provenance::ParseStatus,
        chunks_added: u32,
    },
    DocumentTreeCaptured {
        artifact_id: ArtifactId,
        artifact_version_id: ArtifactVersionId,
        content_hash: ContentHash,
        root_id: StructureNodeId,
        nodes: Vec<StructureNode>,
    },
    PendingIndex {
        artifact_id: ArtifactId,
        content_hash: ContentHash,
    },
    FullTextIndexed {
        artifact_id: ArtifactId,
        chunk_id: ChunkId,
    },
    ArtifactIndexed {
        artifact_id: ArtifactId,
    },
    SearchCompleted {
        artifact_id: ArtifactId,
        cards_added: u32,
    },
    HarnessRunCompleted {
        task_id: Option<TaskId>,
        command: String,
        exit_code: i32,
    },
    ModelAgentProposalRequested {
        request: crate::model_agent::ModelAgentProposalRequest,
    },
    ModelAgentProposalCompleted {
        result: crate::model_agent::ModelAgentProposalResult,
    },
    ApprovalRecorded {
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    },
    MemoryPromoted {
        memory_id: MemoryId,
        candidate_id: MemoryCandidateId,
        security: SecurityMetadata,
    },
    MemoryContradicted {
        memory_id: MemoryId,
        contradicting_candidate_id: MemoryCandidateId,
    },
    MemoryDeprecated {
        memory_id: MemoryId,
    },
    MemorySuperseded {
        memory_id: MemoryId,
        by_memory_id: MemoryId,
    },
    ValidationReportCreated {
        report_id: ValidationReportId,
        task_id: Option<TaskId>,
        passed: bool,
        warnings: Vec<String>,
    },
    TickObserved {
        at: LogicalTick,
    },
    SearchExecuted {
        query: String,
        limit: usize,
        evidence_ids: Vec<EvidenceId>,
        pack_metadata: Option<Box<crate::evidence_pack::EvidencePackMetadataRecord>>,
        at: LogicalTick,
    },
    ParserStarted {
        artifact_id: ArtifactId,
        title: String,
        source_path: String,
        content_hash: ContentHash,
        blob_id: BlobId,
    },

    OcrRequested {
        intent: crate::ocr::OcrIntent,
    },
    OcrCompleted {
        artifact_id: ArtifactId,
        completion: crate::ocr::OcrCompletion,
    },
    OcrFailed {
        artifact_id: ArtifactId,
        request_id: crate::ocr::OcrRequestId,
        reason: String,
    },
    SearchKnowledgeCompleted {
        task_id: Option<TaskId>,
        plan: Option<Box<crate::search::SearchPlan>>,
        outcome: crate::search::SearchOutcome,
    },
    IndexGenerationStarted {
        id: IndexGenerationId,
        name: crate::generations::RepresentationName,
        corpus_snapshot: crate::ids::CorpusSnapshotId,
        fingerprint: crate::generations::IndexFingerprint,
        /// Learned-sparse namespace bound to the generation, when the
        /// representation is the sparse projection.
        sparse_namespace: Option<crate::SparseNamespace>,
    },
    IndexGenerationTransitioned {
        id: IndexGenerationId,
        from: crate::generations::IndexLifecycle,
        to: crate::generations::IndexLifecycle,
        replaced_active_id: Option<IndexGenerationId>,
    },
    SourceBecameStale {
        artifact_id: ArtifactId,
        source_path: String,
        content_hash: ContentHash,
    },
    NotebookCreated {
        notebook_id: crate::ids::NotebookId,
        title: crate::notebook::NotebookTitle,
        created_at: LogicalTick,
        updated_at: LogicalTick,
    },
    NotebookRenamed {
        notebook_id: crate::ids::NotebookId,
        title: crate::notebook::NotebookTitle,
        updated_at: LogicalTick,
    },
    NotebookDeleted {
        notebook_id: crate::ids::NotebookId,
    },
    NotebookSourceAttached {
        notebook_id: crate::ids::NotebookId,
        source_key: crate::notebook::SourceIdentityKey,
        updated_at: LogicalTick,
    },
    NotebookSourceDetached {
        notebook_id: crate::ids::NotebookId,
        source_key: crate::notebook::SourceIdentityKey,
        updated_at: LogicalTick,
    },
    NotebookDraftSaved {
        draft_id: crate::ids::NotebookDraftId,
        notebook_id: crate::ids::NotebookId,
        title: crate::notebook::NotebookDraftTitle,
        body_blob: BlobId,
        body_hash: ContentHash,
        revision: crate::notebook::NotebookDraftRevision,
        citations: Vec<crate::notebook::FrozenNotebookCitation>,
        created_at: LogicalTick,
        updated_at: LogicalTick,
    },
    NotebookDraftDeleted {
        notebook_id: crate::ids::NotebookId,
        draft_id: crate::ids::NotebookDraftId,
        revision: crate::notebook::NotebookDraftRevision,
    },
    RealmReadGrantIssued {
        grant: crate::entities::RealmReadGrant,
    },
    RealmReadGrantRevoked {
        token_digest: crate::GrantTokenDigest,
    },
    FederatedReadAccessRecorded {
        token_digest: crate::GrantTokenDigest,
        provider_realm: crate::RealmId,
        consumer_realm: crate::RealmId,
        record: crate::entities::FederatedAccessRecord,
    },
}

/// Outcome recorded by an `ApprovalRecorded` event.
///
/// `Acknowledged` records an operator decision without a task transition
/// (model-agent approvals); the task linkage is audit metadata only.
/// `TaskTransition` records a decision that transitioned a task; the
/// transition is fully specified, so the old correlated `approved` flag plus
/// `Option` status pair is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Acknowledged {
        task_id: Option<TaskId>,
        approved: bool,
    },
    TaskTransition {
        task_id: TaskId,
        approved: bool,
        from_status: TaskStatus,
        to_status: TaskStatus,
    },
}

impl ApprovalOutcome {
    #[must_use]
    pub const fn approved(self) -> bool {
        match self {
            Self::Acknowledged { approved, .. } | Self::TaskTransition { approved, .. } => approved,
        }
    }

    #[must_use]
    pub const fn task_id(self) -> Option<TaskId> {
        match self {
            Self::Acknowledged { task_id, .. } => task_id,
            Self::TaskTransition { task_id, .. } => Some(task_id),
        }
    }
}
