#![forbid(unsafe_code)]

//! Deterministic domain kernel for Maestria.
//!
//! This module is pure and side-effect free. All environment interaction is
//! represented via `MaestriaEffect` values and executed by a runtime layer.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub const DOMAIN_VERSION: &str = "0.1.0";

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn value(&self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(ArtifactId);
id_type!(ChunkId);
id_type!(CardId);
id_type!(EvidenceId);
id_type!(ClaimId);
id_type!(TaskId);
id_type!(EventId);
id_type!(SequenceNumber);
id_type!(SnapshotId);
id_type!(LogicalTick);
id_type!(RelationId);
id_type!(MemoryCandidateId);
id_type!(MemoryId);
id_type!(ValidationReportId);
id_type!(ApprovalId);
id_type!(HarnessRunId);
id_type!(BlobId);
id_type!(ScopeId);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContentRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub id: ArtifactId,
    pub title: String,
    pub chunk_ids: BTreeSet<ChunkId>,
    pub card_ids: BTreeSet<CardId>,
    pub claim_ids: BTreeSet<ClaimId>,
    pub evidence_ids: BTreeSet<EvidenceId>,
}

impl Artifact {
    pub(crate) fn with_title(id: ArtifactId, title: String) -> Self {
        Self {
            id,
            title,
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: ChunkId,
    pub artifact_id: ArtifactId,
    pub order: u32,
    pub text: String,
}

impl Chunk {
    pub(crate) fn new(id: ChunkId, artifact_id: ArtifactId, order: u32, text: String) -> Self {
        Self {
            id,
            artifact_id,
            order,
            text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    pub artifact_id: ArtifactId,
    pub title: String,
    pub body: String,
    pub claim_ids: BTreeSet<ClaimId>,
}

impl Card {
    pub(crate) fn new(id: CardId, artifact_id: ArtifactId, title: String, body: String) -> Self {
        Self {
            id,
            artifact_id,
            title,
            body,
            claim_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceKind {
    FileSpan {
        path: String,
        range: ContentRange,
        content_hash: String,
        snapshot: Option<BlobId>,
    },
    PdfSpan {
        blob: BlobId,
        page_start: u32,
        page_end: u32,
    },
    WebSnapshot {
        url: String,
        snapshot: BlobId,
        fetched_at: LogicalTick,
        content_hash: String,
    },
    CommandOutput {
        harness_run: HarnessRunId,
        stream: OutputStream,
        blob: BlobId,
    },
    TestResult {
        harness_run: HarnessRunId,
        status: TestStatus,
        log: BlobId,
    },
    Diff {
        harness_run: HarnessRunId,
        patch_blob: BlobId,
    },
    Validation {
        report_id: ValidationReportId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
    Combined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub kind: EvidenceKind,
    pub excerpt: String,
    pub observed_at: LogicalTick,
}

impl Evidence {
    pub(crate) fn new(
        id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
        excerpt: String,
        observed_at: LogicalTick,
    ) -> Self {
        Self {
            id,
            artifact_id,
            claim_id,
            kind,
            excerpt,
            observed_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStatus {
    Draft,
    Proposed,
    Verified,
    Disputed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: ClaimId,
    pub artifact_id: ArtifactId,
    pub text: String,
    pub status: ClaimStatus,
    pub evidence_ids: BTreeSet<EvidenceId>,
}

impl Claim {
    pub(crate) fn new(id: ClaimId, artifact_id: ArtifactId, text: String) -> Self {
        Self {
            id,
            artifact_id,
            text,
            status: ClaimStatus::Draft,
            evidence_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub id: RelationId,
    pub source: RelationEndpoint,
    pub kind: RelationKind,
    pub target: RelationEndpoint,
    pub evidence_id: Option<EvidenceId>,
    pub confidence_milli: u16,
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

pub(crate) const MIN_PROMOTION_CONFIDENCE_MILLI: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReportRecord {
    pub task_id: Option<TaskId>,
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub confidence_milli: u16,
}

impl MemoryCandidate {
    pub fn has_evidence(&self) -> bool {
        !self.evidence_ids.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStatus {
    Active,
    Deprecated,
    Contradicted,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    pub id: MemoryId,
    pub candidate_id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub status: MemoryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Draft,
    Open,
    Active,
    Validating,
    Blocked,
    CompletedVerified,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Draft => matches!(next, Self::Open | Self::Cancelled),
            Self::Open => matches!(next, Self::Active | Self::Cancelled),
            Self::Active => matches!(
                next,
                Self::Validating | Self::Blocked | Self::Failed | Self::Cancelled
            ),
            Self::Validating => matches!(
                next,
                Self::CompletedVerified | Self::CompletedWithWarnings | Self::Failed | Self::Active
            ),
            Self::Blocked => matches!(next, Self::Active | Self::Failed | Self::Cancelled),
            Self::CompletedVerified
            | Self::CompletedWithWarnings
            | Self::Failed
            | Self::Cancelled => false,
        }
    }

    pub fn is_completion(self) -> bool {
        matches!(self, Self::CompletedVerified | Self::CompletedWithWarnings)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub validation_report_id: Option<ValidationReportId>,
    pub artifact_ids: BTreeSet<ArtifactId>,
    pub evidence_ids: BTreeSet<EvidenceId>,
}

impl Task {
    pub(crate) fn new(id: TaskId, title: String, priority: TaskPriority) -> Self {
        Self {
            id,
            title,
            priority,
            status: TaskStatus::Draft,
            validation_report_id: None,
            artifact_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        }
    }
}

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
    },
    ChunkRegistered {
        chunk_id: ChunkId,
        artifact_id: ArtifactId,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: CardId,
        artifact_id: ArtifactId,
        title: String,
        body: String,
    },
    ClaimCreated {
        claim_id: ClaimId,
        artifact_id: ArtifactId,
        text: String,
        evidence_ids: Vec<EvidenceId>,
    },
    EvidenceRecorded {
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
        excerpt: String,
        observed_at: LogicalTick,
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
    },
    MemoryCandidateCreated {
        candidate_id: MemoryCandidateId,
        claim_id: ClaimId,
        evidence_ids: BTreeSet<EvidenceId>,
        confidence_milli: u16,
    },
    UserIntentObserved {
        task_id: TaskId,
        title: String,
    },
    ArtifactParsed {
        artifact_id: ArtifactId,
        chunks_added: u32,
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
        task_id: TaskId,
        approved: bool,
    },
    MemoryPromoted {
        memory_id: MemoryId,
        candidate_id: MemoryCandidateId,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistStateRequest {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreBlobRequest {
    pub artifact_id: ArtifactId,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseArtifactRequest {
    pub artifact_id: ArtifactId,
    pub source_path: String,
    pub source_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFullTextRequest {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexVectorRequest {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateGraphRequest {
    pub relation_id: RelationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchWebRequest {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryHarnessRequest {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub generation: Option<u64>,
    pub capability: String,
    pub scope_id: ScopeId,
    pub approval_id: Option<ApprovalId>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunValidationRequest {
    pub task_id: Option<TaskId>,
    pub claim_id: Option<ClaimId>,
    pub validation_report_id: ValidationReportId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestApprovalRequest {
    pub task_id: TaskId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub task_id: Option<TaskId>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaestriaEffect {
    PersistEvent { envelope: DomainEventEnvelope },
    PersistState(PersistStateRequest),
    StoreBlob(StoreBlobRequest),
    ParseArtifact(ParseArtifactRequest),
    IndexFullText(IndexFullTextRequest),
    IndexVector(IndexVectorRequest),
    UpdateGraph(UpdateGraphRequest),
    QueryHarness(QueryHarnessRequest),
    FetchWeb(FetchWebRequest),
    RunValidation(RunValidationRequest),
    RequestApproval(RequestApprovalRequest),
    EmitDiagnostic(DiagnosticEvent),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KernelOutput {
    pub events: Vec<DomainEventEnvelope>,
    pub effects: Vec<MaestriaEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterArtifactInput {
    pub artifact_id: ArtifactId,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterChunkInput {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub order: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCardInput {
    pub card_id: CardId,
    pub artifact_id: ArtifactId,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordEvidenceInput {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub kind: EvidenceKind,
    pub excerpt: String,
    pub observed_at: LogicalTick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateClaimInput {
    pub claim_id: ClaimId,
    pub artifact_id: ArtifactId,
    pub text: String,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTaskInput {
    pub task_id: TaskId,
    pub title: String,
    pub priority: TaskPriority,
    pub artifact_id: Option<ArtifactId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTaskStatusInput {
    pub task_id: TaskId,
    pub to: TaskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteTaskInput {
    pub task_id: TaskId,
    pub validation_report_id: ValidationReportId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEvidenceToClaimInput {
    pub claim_id: ClaimId,
    pub evidence_id: EvidenceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRelationInput {
    pub relation_id: RelationId,
    pub source: RelationEndpoint,
    pub kind: RelationKind,
    pub target: RelationEndpoint,
    pub evidence_id: Option<EvidenceId>,
    pub confidence_milli: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateMemoryCandidateInput {
    pub candidate_id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: Vec<EvidenceId>,
    pub confidence_milli: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteMemoryInput {
    pub memory_id: MemoryId,
    pub candidate_id: MemoryCandidateId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictMemoryInput {
    pub memory_id: MemoryId,
    pub contradicting_candidate_id: MemoryCandidateId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeprecateMemoryInput {
    pub memory_id: MemoryId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersedeMemoryInput {
    pub memory_id: MemoryId,
    pub by_memory_id: MemoryId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordValidationReportInput {
    pub report_id: ValidationReportId,
    pub task_id: Option<TaskId>,
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserIntent {
    pub task_id: TaskId,
    pub title: String,
    pub priority: TaskPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactDetected {
    pub artifact_id: ArtifactId,
    pub title: String,
    pub source_path: String,
    pub source_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserResult {
    pub artifact_id: ArtifactId,
    pub chunks: Vec<RegisterChunkInput>,
    pub cards: Vec<CreateCardInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResultSet {
    pub artifact_id: ArtifactId,
    pub cards: Vec<CreateCardInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRunCompleted {
    pub task_id: Option<TaskId>,
    pub command: String,
    pub exit_code: i32,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCompleted {
    pub claim_id: ClaimId,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecision {
    pub task_id: TaskId,
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainInput {
    RegisterArtifact(RegisterArtifactInput),
    RegisterChunk(RegisterChunkInput),
    CreateCard(CreateCardInput),
    RecordEvidence(RecordEvidenceInput),
    CreateClaim(CreateClaimInput),
    OpenTask(OpenTaskInput),
    ChangeTaskStatus(ChangeTaskStatusInput),
    CompleteTask(CompleteTaskInput),
    LinkEvidenceToClaim(LinkEvidenceToClaimInput),
    CreateRelation(CreateRelationInput),
    CreateMemoryCandidate(CreateMemoryCandidateInput),
    PromoteMemory(PromoteMemoryInput),
    ContradictMemory(ContradictMemoryInput),
    DeprecateMemory(DeprecateMemoryInput),
    SupersedeMemory(SupersedeMemoryInput),
    RecordValidationReport(RecordValidationReportInput),

    UserIntent(UserIntent),
    ArtifactDetected(ArtifactDetected),
    ParserCompleted(ParserResult),
    SearchCompleted(SearchResultSet),
    HarnessRunCompleted(HarnessRunCompleted),
    ValidationCompleted(ValidationCompleted),
    ApprovalResolved(ApprovalDecision),
    ClockTick(LogicalTick),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    DuplicateId {
        kind: &'static str,
        id: u64,
    },
    MissingArtifact {
        id: ArtifactId,
    },
    MissingChunk {
        id: ChunkId,
    },
    MissingCard {
        id: CardId,
    },
    MissingEvidence {
        id: EvidenceId,
    },
    MissingClaim {
        id: ClaimId,
    },
    MissingTask {
        id: TaskId,
    },
    MissingRelation {
        id: RelationId,
    },
    MissingMemoryCandidate {
        id: MemoryCandidateId,
    },
    MissingMemory {
        id: MemoryId,
    },
    MissingValidationReport {
        id: ValidationReportId,
    },
    ValidationReportTaskMismatch {
        report_id: ValidationReportId,
        report_task_id: Option<TaskId>,
        task_id: TaskId,
    },
    InvalidTaskTransition {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    },
    ValidationRequired {
        task_id: TaskId,
    },
    EvidenceRequired {
        kind: &'static str,
        id: u64,
    },
    MemoryCandidateIneligibleForPromotion {
        candidate_id: MemoryCandidateId,
        confidence_milli: u16,
        minimum_confidence_milli: u16,
        reason: &'static str,
    },
    InvalidEventId {
        expected: u64,
        actual: u64,
    },
    EmptyIntent,
    InvalidSequence {
        expected: u64,
        actual: u64,
    },
    InvalidConfidence {
        max: u16,
        actual: u16,
    },
    ArtifactMismatch {
        expected: ArtifactId,
        actual: ArtifactId,
    },
    ValidationFailed {
        task_id: TaskId,
    },
    ValidationWarningsRequired {
        task_id: TaskId,
    },
    ValidationWarningsForbidden {
        task_id: TaskId,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} id: {id}"),
            Self::MissingArtifact { id } => write!(f, "missing artifact {id}"),
            Self::MissingChunk { id } => write!(f, "missing chunk {id}"),
            Self::MissingCard { id } => write!(f, "missing card {id}"),
            Self::MissingEvidence { id } => write!(f, "missing evidence {id}"),
            Self::MissingClaim { id } => write!(f, "missing claim {id}"),
            Self::MissingTask { id } => write!(f, "missing task {id}"),
            Self::MissingRelation { id } => write!(f, "missing relation {id}"),
            Self::MissingMemoryCandidate { id } => write!(f, "missing memory candidate {id}"),
            Self::MissingMemory { id } => write!(f, "missing memory {id}"),
            Self::MissingValidationReport { id } => write!(f, "missing validation report {id}"),
            Self::ValidationReportTaskMismatch {
                report_id,
                report_task_id,
                task_id,
            } => match report_task_id {
                Some(report_task_id) => write!(
                    f,
                    "validation report {report_id} is for task {report_task_id}, not {task_id}"
                ),
                None => write!(
                    f,
                    "validation report {report_id} is not associated with task {task_id}"
                ),
            },
            Self::InvalidTaskTransition { task_id, from, to } => {
                write!(f, "invalid task transition {task_id}: {from:?} -> {to:?}")
            }
            Self::ValidationRequired { task_id } => {
                write!(f, "task {task_id} requires validation before completion")
            }
            Self::EmptyIntent => write!(f, "user intent must not be empty"),
            Self::EvidenceRequired { kind, id } => {
                write!(f, "{kind} {id} requires at least one evidence id")
            }
            Self::MemoryCandidateIneligibleForPromotion {
                candidate_id,
                confidence_milli,
                minimum_confidence_milli,
                reason,
            } => write!(
                f,
                "memory candidate {candidate_id} cannot be promoted ({reason}): {confidence_milli} < {minimum_confidence_milli}"
            ),
            Self::InvalidSequence { expected, actual } => {
                write!(
                    f,
                    "invalid event sequence: expected {expected}, got {actual}"
                )
            }
            Self::InvalidEventId { expected, actual } => {
                write!(f, "invalid event id: expected {expected}, got {actual}")
            }
            Self::InvalidConfidence { max, actual } => {
                write!(f, "invalid confidence: max {max}, got {actual}")
            }
            Self::ArtifactMismatch { expected, actual } => {
                write!(f, "artifact mismatch: expected {expected}, got {actual}")
            }
            Self::ValidationFailed { task_id } => {
                write!(f, "task {task_id} validation failed")
            }
            Self::ValidationWarningsRequired { task_id } => {
                write!(
                    f,
                    "task {task_id} completed with warnings but validation report has none"
                )
            }
            Self::ValidationWarningsForbidden { task_id } => {
                write!(
                    f,
                    "task {task_id} completed verified but validation report has warnings"
                )
            }
        }
    }
}

impl std::error::Error for DomainError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KernelState {
    pub artifacts: BTreeMap<ArtifactId, Artifact>,
    pub chunks: BTreeMap<ChunkId, Chunk>,
    pub cards: BTreeMap<CardId, Card>,
    pub evidences: BTreeMap<EvidenceId, Evidence>,
    pub claims: BTreeMap<ClaimId, Claim>,
    pub relations: BTreeMap<RelationId, Relation>,
    pub memory_candidates: BTreeMap<MemoryCandidateId, MemoryCandidate>,
    pub memories: BTreeMap<MemoryId, Memory>,
    pub tasks: BTreeMap<TaskId, Task>,
    pub validation_reports: BTreeMap<ValidationReportId, ValidationReportRecord>,
    pub event_log: Vec<DomainEventEnvelope>,
}
