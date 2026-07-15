use crate::entities::{
    ClaimStatus, EvidenceKind, RelationEndpoint, RelationKind, TaskPriority, TaskStatus,
};
use crate::ids::StructureNodeId;
use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, BlobId, CardId, ChunkId, ClaimId, EventId,
    EvidenceId, LogicalTick, MemoryCandidateId, MemoryId, RelationId, SequenceNumber, TaskId,
    ValidationReportId,
};
use crate::search::{ContentHash, StructureNode};
use crate::security::SecurityMetadata;
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
    TaskCompletionRecorded {
        task_id: TaskId,
        status: TaskStatus,
        validation_report_id: ValidationReportId,
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
        content_hash: String,
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
    ApprovalRecorded {
        approval_id: ApprovalId,
        task_id: TaskId,
        approved: bool,
        from_status: Option<TaskStatus>,
        to_status: Option<TaskStatus>,
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
        at: LogicalTick,
    },
    ParserStarted {
        artifact_id: ArtifactId,
        title: String,
        source_path: String,
        content_hash: String,
        blob_id: BlobId,
    },
    SearchKnowledgeCompleted {
        outcome: crate::search::SearchOutcome,
    },
}
