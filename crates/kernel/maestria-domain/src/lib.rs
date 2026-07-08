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
    fn with_title(id: ArtifactId, title: String) -> Self {
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
    fn new(id: ChunkId, artifact_id: ArtifactId, order: u32, text: String) -> Self {
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
    fn new(id: CardId, artifact_id: ArtifactId, title: String, body: String) -> Self {
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
    fn new(
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
    fn new(id: ClaimId, artifact_id: ArtifactId, text: String) -> Self {
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

const MIN_PROMOTION_CONFIDENCE_MILLI: u16 = 500;

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
    fn new(id: TaskId, title: String, priority: TaskPriority) -> Self {
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
    },
    CardCreated {
        card_id: CardId,
        artifact_id: ArtifactId,
    },
    ClaimCreated {
        claim_id: ClaimId,
        artifact_id: ArtifactId,
    },
    EvidenceRecorded {
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
    },
    TaskOpened {
        task_id: TaskId,
        title: String,
        priority: TaskPriority,
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
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseArtifactRequest {
    pub artifact_id: ArtifactId,
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
    PersistEvent { event: DomainEvent },
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
    EmptyIntent,
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

impl KernelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_input(&mut self, input: DomainInput) -> Result<KernelOutput, DomainError> {
        let mut output = KernelOutput::default();

        match input {
            DomainInput::RegisterArtifact(input) => {
                let event = self.handle_register_artifact(input)?;
                let payload = match &event.event {
                    DomainEvent::ArtifactRegistered { title, .. } => title.clone(),
                    _ => String::new(),
                };
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                output
                    .effects
                    .push(MaestriaEffect::StoreBlob(StoreBlobRequest {
                        artifact_id: event_id_to_artifact(&event),
                        payload,
                    }));
            }
            DomainInput::RegisterChunk(input) => {
                let event = self.handle_register_chunk(input.clone())?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                if let DomainEvent::ChunkRegistered {
                    artifact_id,
                    chunk_id,
                    ..
                } = event.event
                {
                    output
                        .effects
                        .push(MaestriaEffect::IndexFullText(IndexFullTextRequest {
                            artifact_id,
                            chunk_id,
                        }));
                    output
                        .effects
                        .push(MaestriaEffect::IndexVector(IndexVectorRequest {
                            artifact_id,
                            chunk_id,
                        }));
                }
            }
            DomainInput::CreateCard(input) => {
                let event = self.handle_create_card(input.clone())?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
            }
            DomainInput::RecordEvidence(input) => {
                let event = self.handle_record_evidence(input.clone())?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                let claim_id = match event.event {
                    DomainEvent::EvidenceRecorded { claim_id, .. } => claim_id,
                    _ => None,
                };
                if let Some(claim_id) = claim_id {
                    output
                        .effects
                        .push(MaestriaEffect::RunValidation(RunValidationRequest {
                            task_id: None,
                            claim_id: Some(claim_id),
                            validation_report_id: ValidationReportId::new(0),
                        }));
                }
            }
            DomainInput::CreateClaim(input) => {
                let event = self.handle_create_claim(input.clone())?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                output
                    .effects
                    .push(MaestriaEffect::RunValidation(RunValidationRequest {
                        task_id: None,
                        claim_id: Some(input.claim_id),
                        validation_report_id: ValidationReportId::new(0),
                    }));
            }
            DomainInput::OpenTask(input) => {
                let event = self.handle_open_task(input.clone())?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                if input.priority == TaskPriority::High {
                    let task_id = match event.event {
                        DomainEvent::TaskOpened { task_id, .. } => task_id,
                        _ => input.task_id,
                    };
                    output
                        .effects
                        .push(MaestriaEffect::RequestApproval(RequestApprovalRequest {
                            task_id,
                        }));
                }
            }
            DomainInput::ChangeTaskStatus(input) => {
                let (from, to) = self.handle_change_task_status(input.task_id, input.to)?;
                let event = self.emit_event(DomainEvent::TaskStatusChanged {
                    task_id: input.task_id,
                    from,
                    to,
                });
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
            }
            DomainInput::CompleteTask(input) => {
                let event = self.handle_complete_task(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                output
                    .effects
                    .push(MaestriaEffect::PersistState(PersistStateRequest {
                        reason: "validated task completion".to_string(),
                    }));
            }
            DomainInput::LinkEvidenceToClaim(input) => {
                let claim_id = input.claim_id;
                let event = self.handle_link_evidence_to_claim(input.clone())?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
                output
                    .effects
                    .push(MaestriaEffect::RunValidation(RunValidationRequest {
                        task_id: None,
                        claim_id: Some(claim_id),
                        validation_report_id: ValidationReportId::new(0),
                    }));
            }
            DomainInput::CreateRelation(input) => {
                let event = self.handle_create_relation(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
                if let DomainEvent::RelationCreated { relation_id } = event.event {
                    output
                        .effects
                        .push(MaestriaEffect::UpdateGraph(UpdateGraphRequest {
                            relation_id,
                        }));
                }
            }
            DomainInput::CreateMemoryCandidate(input) => {
                let event = self.handle_create_memory_candidate(input)?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
            }
            DomainInput::PromoteMemory(input) => {
                let event = self.handle_promote_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
            }
            DomainInput::ContradictMemory(input) => {
                let event = self.handle_contradict_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
            }
            DomainInput::DeprecateMemory(input) => {
                let event = self.handle_deprecate_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
            }
            DomainInput::SupersedeMemory(input) => {
                let event = self.handle_supersede_memory(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
            }
            DomainInput::RecordValidationReport(input) => {
                let event = self.handle_record_validation_report(input)?;
                output.events.push(event.clone());
                output.effects.push(MaestriaEffect::PersistEvent {
                    event: event.event.clone(),
                });
            }
            DomainInput::UserIntent(input) => {
                let event = self.handle_user_intent(input.clone())?;
                for entry in event {
                    output.events.push(entry.clone());
                    output
                        .effects
                        .push(MaestriaEffect::PersistEvent { event: entry.event });
                }
            }
            DomainInput::ArtifactDetected(input) => {
                let cmd = RegisterArtifactInput {
                    artifact_id: input.artifact_id,
                    title: input.title,
                };
                let event = self.handle_register_artifact(cmd)?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
            }
            DomainInput::ParserCompleted(input) => {
                let generated = self.handle_parser_completed(input)?;
                for envelope in generated {
                    output.events.push(envelope.clone());
                    output.effects.push(MaestriaEffect::PersistEvent {
                        event: envelope.event,
                    });
                }
            }
            DomainInput::SearchCompleted(input) => {
                let generated = self.handle_search_completed(input)?;
                for envelope in generated {
                    output.events.push(envelope.clone());
                    output.effects.push(MaestriaEffect::PersistEvent {
                        event: envelope.event,
                    });
                }
            }
            DomainInput::HarnessRunCompleted(input) => {
                let generated = self.handle_harness_completed(input)?;
                for envelope in generated {
                    output.events.push(envelope.clone());
                    output.effects.push(MaestriaEffect::PersistEvent {
                        event: envelope.event,
                    });
                }
            }
            DomainInput::ValidationCompleted(input) => {
                let event = self.handle_validation_completed(input)?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
            }
            DomainInput::ApprovalResolved(input) => {
                let envelopes = self.handle_approval_resolved(input)?;
                for envelope in envelopes {
                    output.events.push(envelope.clone());
                    output.effects.push(MaestriaEffect::PersistEvent {
                        event: envelope.event,
                    });
                }
            }
            DomainInput::ClockTick(tick) => {
                let event = self.emit_event(DomainEvent::TickObserved { at: tick });
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
            }
        }

        Ok(output)
    }

    fn emit_event(&mut self, event: DomainEvent) -> DomainEventEnvelope {
        let id = EventId(self.event_log.len() as u64 + 1);
        let sequence = SequenceNumber(id.value());
        let envelope = DomainEventEnvelope {
            id,
            sequence,
            event,
        };
        self.event_log.push(envelope.clone());
        envelope
    }

    fn handle_register_artifact(
        &mut self,
        input: RegisterArtifactInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::DuplicateId {
                kind: "artifact",
                id: input.artifact_id.value(),
            });
        }
        self.artifacts.insert(
            input.artifact_id,
            Artifact::with_title(input.artifact_id, input.title.clone()),
        );
        Ok(self.emit_event(DomainEvent::ArtifactRegistered {
            artifact_id: input.artifact_id,
            title: input.title,
        }))
    }

    fn handle_register_chunk(
        &mut self,
        input: RegisterChunkInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if self.chunks.contains_key(&input.chunk_id) {
            return Err(DomainError::DuplicateId {
                kind: "chunk",
                id: input.chunk_id.value(),
            });
        }
        if self
            .chunks
            .values()
            .any(|chunk| chunk.artifact_id == input.artifact_id && chunk.order == input.order)
        {
            return Err(DomainError::DuplicateId {
                kind: "chunk_order",
                id: input.chunk_id.value(),
            });
        }

        let chunk = Chunk::new(input.chunk_id, input.artifact_id, input.order, input.text);
        self.chunks.insert(input.chunk_id, chunk);
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.chunk_ids.insert(input.chunk_id);
        }

        Ok(self.emit_event(DomainEvent::ChunkRegistered {
            chunk_id: input.chunk_id,
            artifact_id: input.artifact_id,
            order: input.order,
        }))
    }

    fn handle_create_card(
        &mut self,
        input: CreateCardInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.cards.contains_key(&input.card_id) {
            return Err(DomainError::DuplicateId {
                kind: "card",
                id: input.card_id.value(),
            });
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        self.cards.insert(
            input.card_id,
            Card::new(input.card_id, input.artifact_id, input.title, input.body),
        );

        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.card_ids.insert(input.card_id);
        }

        Ok(self.emit_event(DomainEvent::CardCreated {
            card_id: input.card_id,
            artifact_id: input.artifact_id,
        }))
    }

    fn handle_record_evidence(
        &mut self,
        input: RecordEvidenceInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.evidences.contains_key(&input.evidence_id) {
            return Err(DomainError::DuplicateId {
                kind: "evidence",
                id: input.evidence_id.value(),
            });
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if let Some(claim_id) = input.claim_id
            && !self.claims.contains_key(&claim_id)
        {
            return Err(DomainError::MissingClaim { id: claim_id });
        }

        let kind = input.kind.clone();
        self.evidences.insert(
            input.evidence_id,
            Evidence::new(
                input.evidence_id,
                input.artifact_id,
                input.claim_id,
                kind.clone(),
                input.excerpt,
                input.observed_at,
            ),
        );

        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.evidence_ids.insert(input.evidence_id);
        }
        if let Some(claim_id) = input.claim_id
            && let Some(claim) = self.claims.get_mut(&claim_id)
        {
            claim.evidence_ids.insert(input.evidence_id);
        }

        Ok(self.emit_event(DomainEvent::EvidenceRecorded {
            evidence_id: input.evidence_id,
            artifact_id: input.artifact_id,
            claim_id: input.claim_id,
            kind,
        }))
    }

    fn handle_create_claim(
        &mut self,
        input: CreateClaimInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.claims.contains_key(&input.claim_id) {
            return Err(DomainError::DuplicateId {
                kind: "claim",
                id: input.claim_id.value(),
            });
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut claim = Claim::new(input.claim_id, input.artifact_id, input.text);
        for evidence_id in input.evidence_ids {
            if !self.evidences.contains_key(&evidence_id) {
                return Err(DomainError::MissingEvidence { id: evidence_id });
            }
            claim.evidence_ids.insert(evidence_id);
            if let Some(evidence) = self.evidences.get_mut(&evidence_id) {
                evidence.claim_id = Some(input.claim_id);
            }
        }

        self.claims.insert(input.claim_id, claim);
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.claim_ids.insert(input.claim_id);
        }

        Ok(self.emit_event(DomainEvent::ClaimCreated {
            claim_id: input.claim_id,
            artifact_id: input.artifact_id,
        }))
    }

    fn handle_open_task(
        &mut self,
        input: OpenTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.tasks.contains_key(&input.task_id) {
            return Err(DomainError::DuplicateId {
                kind: "task",
                id: input.task_id.value(),
            });
        }
        if let Some(artifact_id) = input.artifact_id
            && !self.artifacts.contains_key(&artifact_id)
        {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }

        let task = Task::new(input.task_id, input.title.clone(), input.priority);
        let artifact_id = input.artifact_id;
        self.tasks.insert(input.task_id, task);
        if let Some(artifact_id) = artifact_id
            && let Some(task) = self.tasks.get_mut(&input.task_id)
        {
            task.artifact_ids.insert(artifact_id);
        }

        Ok(self.emit_event(DomainEvent::TaskOpened {
            task_id: input.task_id,
            title: input.title,
            priority: input.priority,
        }))
    }

    fn handle_change_task_status(
        &mut self,
        task_id: TaskId,
        to: TaskStatus,
    ) -> Result<(TaskStatus, TaskStatus), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        let from = task.status;
        if to.is_completion() {
            return Err(DomainError::ValidationRequired { task_id });
        }
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition { task_id, from, to });
        }
        task.status = to;
        Ok((from, to))
    }

    fn handle_complete_task(
        &mut self,
        input: CompleteTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let task = self
            .tasks
            .get_mut(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;
        let report = self
            .validation_reports
            .get(&input.validation_report_id)
            .ok_or(DomainError::MissingValidationReport {
                id: input.validation_report_id,
            })?;
        if report.task_id != Some(input.task_id) {
            return Err(DomainError::ValidationReportTaskMismatch {
                report_id: input.validation_report_id,
                report_task_id: report.task_id,
                task_id: input.task_id,
            });
        }
        let from = task.status;
        let to = if report.warnings.is_empty() {
            TaskStatus::CompletedVerified
        } else {
            TaskStatus::CompletedWithWarnings
        };
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition {
                task_id: input.task_id,
                from,
                to,
            });
        }
        task.status = to;
        task.validation_report_id = Some(input.validation_report_id);
        Ok(self.emit_event(DomainEvent::TaskCompletionRecorded {
            task_id: input.task_id,
            status: to,
            validation_report_id: input.validation_report_id,
        }))
    }

    fn handle_link_evidence_to_claim(
        &mut self,
        input: LinkEvidenceToClaimInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let claim = self
            .claims
            .get_mut(&input.claim_id)
            .ok_or(DomainError::MissingClaim { id: input.claim_id })?;
        if !self.evidences.contains_key(&input.evidence_id) {
            return Err(DomainError::MissingEvidence {
                id: input.evidence_id,
            });
        }

        claim.evidence_ids.insert(input.evidence_id);
        if let Some(evidence) = self.evidences.get_mut(&input.evidence_id) {
            evidence.claim_id = Some(input.claim_id);
        }

        Ok(self.emit_event(DomainEvent::ClaimEvidenceLinked {
            claim_id: input.claim_id,
            evidence_id: input.evidence_id,
        }))
    }

    fn handle_create_relation(
        &mut self,
        input: CreateRelationInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.relations.contains_key(&input.relation_id) {
            return Err(DomainError::DuplicateId {
                kind: "relation",
                id: input.relation_id.value(),
            });
        }
        if let Some(evidence_id) = input.evidence_id
            && !self.evidences.contains_key(&evidence_id)
        {
            return Err(DomainError::MissingEvidence { id: evidence_id });
        }
        let relation = Relation {
            id: input.relation_id,
            source: input.source,
            kind: input.kind,
            target: input.target,
            evidence_id: input.evidence_id,
            confidence_milli: input.confidence_milli.min(1000),
        };
        self.relations.insert(input.relation_id, relation);
        Ok(self.emit_event(DomainEvent::RelationCreated {
            relation_id: input.relation_id,
        }))
    }

    fn handle_create_memory_candidate(
        &mut self,
        input: CreateMemoryCandidateInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.memory_candidates.contains_key(&input.candidate_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory_candidate",
                id: input.candidate_id.value(),
            });
        }
        if !self.claims.contains_key(&input.claim_id) {
            return Err(DomainError::MissingClaim { id: input.claim_id });
        }
        let mut evidence_ids = BTreeSet::new();
        for evidence_id in input.evidence_ids {
            if !self.evidences.contains_key(&evidence_id) {
                return Err(DomainError::MissingEvidence { id: evidence_id });
            }
            evidence_ids.insert(evidence_id);
        }
        if evidence_ids.is_empty() {
            return Err(DomainError::EvidenceRequired {
                kind: "memory_candidate",
                id: input.candidate_id.value(),
            });
        }
        let candidate = MemoryCandidate {
            id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids: evidence_ids.clone(),
            confidence_milli: input.confidence_milli.min(1000),
        };
        self.memory_candidates.insert(input.candidate_id, candidate);
        Ok(self.emit_event(DomainEvent::MemoryCandidateCreated {
            candidate_id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids,
            confidence_milli: input.confidence_milli.min(1000),
        }))
    }

    fn handle_promote_memory(
        &mut self,
        input: PromoteMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let candidate = self.memory_candidates.get(&input.candidate_id).ok_or(
            DomainError::MissingMemoryCandidate {
                id: input.candidate_id,
            },
        )?;
        if candidate.evidence_ids.is_empty() {
            return Err(DomainError::MemoryCandidateIneligibleForPromotion {
                candidate_id: candidate.id,
                confidence_milli: candidate.confidence_milli,
                minimum_confidence_milli: MIN_PROMOTION_CONFIDENCE_MILLI,
                reason: "no evidence ids",
            });
        }
        if !candidate
            .evidence_ids
            .iter()
            .all(|evidence_id| self.evidences.contains_key(evidence_id))
        {
            return Err(DomainError::MemoryCandidateIneligibleForPromotion {
                candidate_id: candidate.id,
                confidence_milli: candidate.confidence_milli,
                minimum_confidence_milli: MIN_PROMOTION_CONFIDENCE_MILLI,
                reason: "missing evidence",
            });
        }
        if candidate.confidence_milli < MIN_PROMOTION_CONFIDENCE_MILLI {
            return Err(DomainError::MemoryCandidateIneligibleForPromotion {
                candidate_id: candidate.id,
                confidence_milli: candidate.confidence_milli,
                minimum_confidence_milli: MIN_PROMOTION_CONFIDENCE_MILLI,
                reason: "insufficient confidence",
            });
        }
        if self.memories.contains_key(&input.memory_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory",
                id: input.memory_id.value(),
            });
        }

        let memory = Memory {
            id: input.memory_id,
            candidate_id: input.candidate_id,
            claim_id: candidate.claim_id,
            evidence_ids: candidate.evidence_ids.clone(),
            status: MemoryStatus::Active,
        };
        self.memories.insert(input.memory_id, memory);

        Ok(self.emit_event(DomainEvent::MemoryPromoted {
            memory_id: input.memory_id,
            candidate_id: input.candidate_id,
        }))
    }

    fn handle_contradict_memory(
        &mut self,
        input: ContradictMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self
            .memory_candidates
            .contains_key(&input.contradicting_candidate_id)
        {
            return Err(DomainError::MissingMemoryCandidate {
                id: input.contradicting_candidate_id,
            });
        }
        let memory = self
            .memories
            .get_mut(&input.memory_id)
            .ok_or(DomainError::MissingMemory {
                id: input.memory_id,
            })?;
        memory.status = MemoryStatus::Contradicted;

        Ok(self.emit_event(DomainEvent::MemoryContradicted {
            memory_id: input.memory_id,
            contradicting_candidate_id: input.contradicting_candidate_id,
        }))
    }

    fn handle_deprecate_memory(
        &mut self,
        input: DeprecateMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let memory = self
            .memories
            .get_mut(&input.memory_id)
            .ok_or(DomainError::MissingMemory {
                id: input.memory_id,
            })?;
        memory.status = MemoryStatus::Deprecated;

        Ok(self.emit_event(DomainEvent::MemoryDeprecated {
            memory_id: input.memory_id,
        }))
    }

    fn handle_supersede_memory(
        &mut self,
        input: SupersedeMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self.memories.contains_key(&input.by_memory_id) {
            return Err(DomainError::MissingMemory {
                id: input.by_memory_id,
            });
        }
        let memory = self
            .memories
            .get_mut(&input.memory_id)
            .ok_or(DomainError::MissingMemory {
                id: input.memory_id,
            })?;
        memory.status = MemoryStatus::Superseded;

        Ok(self.emit_event(DomainEvent::MemorySuperseded {
            memory_id: input.memory_id,
            by_memory_id: input.by_memory_id,
        }))
    }

    fn handle_record_validation_report(
        &mut self,
        input: RecordValidationReportInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.validation_reports.contains_key(&input.report_id) {
            return Err(DomainError::DuplicateId {
                kind: "validation_report",
                id: input.report_id.value(),
            });
        }
        if let Some(task_id) = input.task_id
            && !self.tasks.contains_key(&task_id)
        {
            return Err(DomainError::MissingTask { id: task_id });
        }
        self.validation_reports.insert(
            input.report_id,
            ValidationReportRecord {
                task_id: input.task_id,
                passed: input.passed,
                warnings: input.warnings.clone(),
            },
        );
        Ok(self.emit_event(DomainEvent::ValidationReportCreated {
            report_id: input.report_id,
            task_id: input.task_id,
            passed: input.passed,
            warnings: input.warnings,
        }))
    }

    fn handle_user_intent(
        &mut self,
        input: UserIntent,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if input.title.trim().is_empty() {
            return Err(DomainError::EmptyIntent);
        }

        let open = self.handle_open_task(OpenTaskInput {
            task_id: input.task_id,
            title: input.title.clone(),
            priority: input.priority,
            artifact_id: None,
        })?;

        let observed = self.emit_event(DomainEvent::UserIntentObserved {
            task_id: input.task_id,
            title: input.title,
        });

        Ok(vec![open, observed])
    }

    fn handle_parser_completed(
        &mut self,
        input: ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut generated = Vec::new();
        for chunk in input.chunks {
            generated.push(self.handle_register_chunk(chunk)?);
        }

        let chunks_added = (generated.len().min(u32::MAX as usize)) as u32;
        for card in input.cards {
            generated.push(self.handle_create_card(card)?);
        }

        let parsed = self.emit_event(DomainEvent::ArtifactParsed {
            artifact_id: input.artifact_id,
            chunks_added,
        });
        generated.push(parsed);
        Ok(generated)
    }

    fn handle_search_completed(
        &mut self,
        input: SearchResultSet,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut generated = Vec::new();
        for card in input.cards {
            generated.push(self.handle_create_card(card)?);
        }

        let cards_added = (generated.len().min(u32::MAX as usize)) as u32;
        let event = self.emit_event(DomainEvent::SearchCompleted {
            artifact_id: input.artifact_id,
            cards_added,
        });
        generated.push(event);
        Ok(generated)
    }

    fn handle_harness_completed(
        &mut self,
        input: HarnessRunCompleted,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();
        let task_id = input.task_id;
        let exit_code = input.exit_code;

        let base_event = self.emit_event(DomainEvent::HarnessRunCompleted {
            task_id,
            command: input.command,
            exit_code,
        });
        generated.push(base_event);

        if let Some(task_id) = task_id
            && let Some(task) = self.tasks.get(&task_id)
        {
            if input.exit_code != 0 && task.status.can_transition_to(TaskStatus::Blocked) {
                let (from, to) = self.handle_change_task_status(task_id, TaskStatus::Blocked)?;
                generated.push(self.emit_event(DomainEvent::TaskStatusChanged {
                    task_id,
                    from,
                    to,
                }));
            } else if input.exit_code == 0 && task.status == TaskStatus::Draft {
                let (from, to) = self.handle_change_task_status(task_id, TaskStatus::Open)?;
                generated.push(self.emit_event(DomainEvent::TaskStatusChanged {
                    task_id,
                    from,
                    to,
                }));
            }
        }

        if input.exit_code != 0
            && let Some(task_id) = task_id
        {
            generated.push(self.emit_event(DomainEvent::ApprovalRecorded {
                task_id,
                approved: false,
            }));
        }

        Ok(generated)
    }

    fn handle_validation_completed(
        &mut self,
        input: ValidationCompleted,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let status = if input.valid {
            ClaimStatus::Verified
        } else {
            ClaimStatus::Disputed
        };

        let claim = self
            .claims
            .get_mut(&input.claim_id)
            .ok_or(DomainError::MissingClaim { id: input.claim_id })?;
        claim.status = status.clone();

        Ok(self.emit_event(DomainEvent::ClaimValidationUpdated {
            claim_id: input.claim_id,
            status,
        }))
    }

    fn handle_approval_resolved(
        &mut self,
        input: ApprovalDecision,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let task = self
            .tasks
            .get(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;

        let target = if input.approved {
            match task.status {
                TaskStatus::Draft | TaskStatus::Open | TaskStatus::Blocked => TaskStatus::Active,
                _ => task.status,
            }
        } else {
            TaskStatus::Blocked
        };
        let (from, to) = if task.status == target {
            (task.status, task.status)
        } else {
            self.handle_change_task_status(input.task_id, target)?
        };
        let emitted = vec![
            self.emit_event(DomainEvent::TaskStatusChanged {
                task_id: input.task_id,
                from,
                to,
            }),
            self.emit_event(DomainEvent::ApprovalRecorded {
                task_id: input.task_id,
                approved: input.approved,
            }),
        ];

        Ok(emitted)
    }

    pub fn apply_event(&mut self, envelope: DomainEventEnvelope) -> Result<(), DomainError> {
        match &envelope.event {
            DomainEvent::ArtifactRegistered { artifact_id, title } => {
                if !self.artifacts.contains_key(artifact_id) {
                    self.artifacts.insert(
                        *artifact_id,
                        Artifact::with_title(*artifact_id, title.clone()),
                    );
                }
            }
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.chunks.contains_key(chunk_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "chunk",
                        id: chunk_id.value(),
                    });
                }
                self.chunks.insert(
                    *chunk_id,
                    Chunk::new(*chunk_id, *artifact_id, *order, String::new()),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.chunk_ids.insert(*chunk_id);
                }
            }
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.cards.contains_key(card_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "card",
                        id: card_id.value(),
                    });
                }
                self.cards.insert(
                    *card_id,
                    Card::new(*card_id, *artifact_id, String::new(), String::new()),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.card_ids.insert(*card_id);
                }
            }
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.claims.contains_key(claim_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "claim",
                        id: claim_id.value(),
                    });
                }
                self.claims.insert(
                    *claim_id,
                    Claim::new(*claim_id, *artifact_id, String::new()),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.claim_ids.insert(*claim_id);
                }
            }
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.evidences.contains_key(evidence_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "evidence",
                        id: evidence_id.value(),
                    });
                }
                if let Some(claim_id) = claim_id
                    && !self.claims.contains_key(claim_id)
                {
                    return Err(DomainError::MissingClaim { id: *claim_id });
                }

                self.evidences.insert(
                    *evidence_id,
                    Evidence::new(
                        *evidence_id,
                        *artifact_id,
                        *claim_id,
                        kind.clone(),
                        String::new(),
                        LogicalTick::new(0),
                    ),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.evidence_ids.insert(*evidence_id);
                }
                if let Some(claim_id) = claim_id
                    && let Some(claim) = self.claims.get_mut(claim_id)
                {
                    claim.evidence_ids.insert(*evidence_id);
                }
            }
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
            } => {
                if self.tasks.contains_key(task_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "task",
                        id: task_id.value(),
                    });
                }
                self.tasks
                    .insert(*task_id, Task::new(*task_id, title.clone(), *priority));
            }
            DomainEvent::TaskStatusChanged {
                task_id,
                from: _,
                to,
            } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(DomainError::MissingTask { id: *task_id })?;
                task.status = *to;
            }
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(DomainError::MissingTask { id: *task_id })?;
                task.status = *status;
                task.validation_report_id = Some(*validation_report_id);
            }
            DomainEvent::ClaimValidationUpdated { claim_id, status } => {
                let claim = self
                    .claims
                    .get_mut(claim_id)
                    .ok_or(DomainError::MissingClaim { id: *claim_id })?;
                claim.status = status.clone();
            }
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => {
                let claim = self
                    .claims
                    .get_mut(claim_id)
                    .ok_or(DomainError::MissingClaim { id: *claim_id })?;
                if !self.evidences.contains_key(evidence_id) {
                    return Err(DomainError::MissingEvidence { id: *evidence_id });
                }
                claim.evidence_ids.insert(*evidence_id);
                if let Some(evidence) = self.evidences.get_mut(evidence_id) {
                    evidence.claim_id = Some(*claim_id);
                }
            }
            DomainEvent::RelationCreated { relation_id } => {
                if self.relations.contains_key(relation_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "relation",
                        id: relation_id.value(),
                    });
                }
                self.relations.insert(
                    *relation_id,
                    Relation {
                        id: *relation_id,
                        source: RelationEndpoint::Artifact(ArtifactId::new(0)),
                        kind: RelationKind::RelatedTo,
                        target: RelationEndpoint::Artifact(ArtifactId::new(0)),
                        evidence_id: None,
                        confidence_milli: 0,
                    },
                );
            }
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => {
                if self.memory_candidates.contains_key(candidate_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "memory_candidate",
                        id: candidate_id.value(),
                    });
                }
                if !self.claims.contains_key(claim_id) {
                    return Err(DomainError::MissingClaim { id: *claim_id });
                }
                if evidence_ids.is_empty() {
                    return Err(DomainError::EvidenceRequired {
                        kind: "memory_candidate",
                        id: candidate_id.value(),
                    });
                }
                for evidence_id in evidence_ids {
                    if !self.evidences.contains_key(evidence_id) {
                        return Err(DomainError::MissingEvidence { id: *evidence_id });
                    }
                }
                self.memory_candidates.insert(
                    *candidate_id,
                    MemoryCandidate {
                        id: *candidate_id,
                        claim_id: *claim_id,
                        evidence_ids: evidence_ids.clone(),
                        confidence_milli: *confidence_milli,
                    },
                );
            }
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
            } => {
                if self.memories.contains_key(memory_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "memory",
                        id: memory_id.value(),
                    });
                }
                let candidate = self
                    .memory_candidates
                    .get(candidate_id)
                    .ok_or(DomainError::MissingMemoryCandidate { id: *candidate_id })?;
                self.memories.insert(
                    *memory_id,
                    Memory {
                        id: *memory_id,
                        candidate_id: *candidate_id,
                        claim_id: candidate.claim_id,
                        evidence_ids: candidate.evidence_ids.clone(),
                        status: MemoryStatus::Active,
                    },
                );
            }
            DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => {
                if !self
                    .memory_candidates
                    .contains_key(contradicting_candidate_id)
                {
                    return Err(DomainError::MissingMemoryCandidate {
                        id: *contradicting_candidate_id,
                    });
                }
                let memory = self
                    .memories
                    .get_mut(memory_id)
                    .ok_or(DomainError::MissingMemory { id: *memory_id })?;
                memory.status = MemoryStatus::Contradicted;
            }
            DomainEvent::MemoryDeprecated { memory_id } => {
                let memory = self
                    .memories
                    .get_mut(memory_id)
                    .ok_or(DomainError::MissingMemory { id: *memory_id })?;
                memory.status = MemoryStatus::Deprecated;
            }
            DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => {
                if !self.memories.contains_key(by_memory_id) {
                    return Err(DomainError::MissingMemory { id: *by_memory_id });
                }
                let memory = self
                    .memories
                    .get_mut(memory_id)
                    .ok_or(DomainError::MissingMemory { id: *memory_id })?;
                memory.status = MemoryStatus::Superseded;
            }
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => {
                if self.validation_reports.contains_key(report_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "validation_report",
                        id: report_id.value(),
                    });
                }
                self.validation_reports.insert(
                    *report_id,
                    ValidationReportRecord {
                        task_id: *task_id,
                        passed: *passed,
                        warnings: warnings.clone(),
                    },
                );
            }
            DomainEvent::UserIntentObserved { .. }
            | DomainEvent::ArtifactParsed { .. }
            | DomainEvent::SearchCompleted { .. }
            | DomainEvent::HarnessRunCompleted { .. }
            | DomainEvent::ApprovalRecorded { .. }
            | DomainEvent::TickObserved { .. } => {}
        }

        self.event_log.push(envelope);
        Ok(())
    }
}

fn event_id_to_artifact(event: &DomainEventEnvelope) -> ArtifactId {
    match event.event {
        DomainEvent::ArtifactRegistered { artifact_id, .. } => artifact_id,
        DomainEvent::ChunkRegistered { artifact_id, .. } => artifact_id,
        DomainEvent::CardCreated { artifact_id, .. } => artifact_id,
        DomainEvent::ClaimCreated { artifact_id, .. } => artifact_id,
        DomainEvent::EvidenceRecorded { artifact_id, .. } => artifact_id,
        _ => ArtifactId::new(0),
    }
}

/// Replay a deterministic input sequence into a fresh state.
pub fn replay_inputs(
    inputs: &[DomainInput],
) -> Result<(KernelState, Vec<DomainEventEnvelope>, Vec<MaestriaEffect>), DomainError> {
    let mut state = KernelState::new();
    let mut events = Vec::new();
    let mut effects = Vec::new();

    for input in inputs {
        let output = state.apply_input(input.clone())?;
        events.extend(output.events);
        effects.extend(output.effects);
    }

    Ok((state, events, effects))
}

/// Replay a deterministic event log into a fresh state.
pub fn replay_events(envelopes: &[DomainEventEnvelope]) -> Result<KernelState, DomainError> {
    let mut state = KernelState::new();
    for envelope in envelopes {
        state.apply_event(envelope.clone())?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_inputs() -> Vec<DomainInput> {
        vec![
            DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id: ArtifactId::new(1),
                title: "Project Notes".to_string(),
            }),
            DomainInput::ParserCompleted(ParserResult {
                artifact_id: ArtifactId::new(1),
                chunks: vec![
                    RegisterChunkInput {
                        chunk_id: ChunkId::new(10),
                        artifact_id: ArtifactId::new(1),
                        order: 0,
                        text: "first chunk".to_string(),
                    },
                    RegisterChunkInput {
                        chunk_id: ChunkId::new(11),
                        artifact_id: ArtifactId::new(1),
                        order: 1,
                        text: "second chunk".to_string(),
                    },
                ],
                cards: Vec::new(),
            }),
            DomainInput::CreateClaim(CreateClaimInput {
                claim_id: ClaimId::new(20),
                artifact_id: ArtifactId::new(1),
                text: "Claim from evidence".to_string(),
                evidence_ids: Vec::new(),
            }),
            DomainInput::CreateCard(CreateCardInput {
                card_id: CardId::new(30),
                artifact_id: ArtifactId::new(1),
                title: "Summary".to_string(),
                body: "Summarize project notes".to_string(),
            }),
            DomainInput::RecordEvidence(RecordEvidenceInput {
                evidence_id: EvidenceId::new(40),
                artifact_id: ArtifactId::new(1),
                claim_id: Some(ClaimId::new(20)),
                kind: EvidenceKind::FileSpan {
                    path: "notes.txt".to_string(),
                    range: ContentRange { start: 1, end: 2 },
                    content_hash: "sha256:notes".to_string(),
                },
                excerpt: "first chunk".to_string(),
                observed_at: LogicalTick::new(12),
            }),
            DomainInput::LinkEvidenceToClaim(LinkEvidenceToClaimInput {
                claim_id: ClaimId::new(20),
                evidence_id: EvidenceId::new(40),
            }),
            DomainInput::UserIntent(UserIntent {
                task_id: TaskId::new(50),
                title: "Summarize artifact".to_string(),
                priority: TaskPriority::Normal,
            }),
            DomainInput::ValidationCompleted(ValidationCompleted {
                claim_id: ClaimId::new(20),
                valid: true,
            }),
            DomainInput::ClockTick(LogicalTick::new(99)),
        ]
    }

    fn run_replay_once()
    -> Result<(KernelState, Vec<DomainEventEnvelope>, Vec<MaestriaEffect>), DomainError> {
        replay_inputs(&sample_inputs())
    }

    fn register_artifact_and_claim(state: &mut KernelState) -> Result<(), DomainError> {
        state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
            artifact_id: ArtifactId::new(1),
            title: "Project Notes".to_string(),
        }))?;
        state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
            claim_id: ClaimId::new(20),
            artifact_id: ArtifactId::new(1),
            text: "Claim from evidence".to_string(),
            evidence_ids: Vec::new(),
        }))?;
        Ok(())
    }

    fn file_span_kind() -> EvidenceKind {
        EvidenceKind::FileSpan {
            path: "notes.txt".to_string(),
            range: ContentRange { start: 1, end: 2 },
            content_hash: "sha256:notes".to_string(),
        }
    }

    fn state_with_memory_candidate(
        candidate_id: MemoryCandidateId,
    ) -> Result<KernelState, DomainError> {
        let mut state = KernelState::new();
        register_artifact_and_claim(&mut state)?;
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: file_span_kind(),
            excerpt: "first chunk".to_string(),
            observed_at: LogicalTick::new(12),
        }))?;
        state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id,
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40)],
                confidence_milli: 720,
            },
        ))?;
        Ok(state)
    }

    fn promote_memory(
        state: &mut KernelState,
        memory_id: MemoryId,
        candidate_id: MemoryCandidateId,
    ) -> Result<(), DomainError> {
        state.apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
            memory_id,
            candidate_id,
        }))?;
        Ok(())
    }

    #[test]
    fn parser_completed_registers_chunks_and_cards() -> Result<(), DomainError> {
        let mut state = KernelState::new();
        state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
            artifact_id: ArtifactId::new(1),
            title: "Project Notes".to_string(),
        }))?;

        let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
            artifact_id: ArtifactId::new(1),
            chunks: vec![RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                order: 0,
                text: "first chunk".to_string(),
            }],
            cards: vec![CreateCardInput {
                card_id: CardId::new(20),
                artifact_id: ArtifactId::new(1),
                title: "Summary".to_string(),
                body: "Parsed summary".to_string(),
            }],
        }))?;

        assert!(state.chunks.contains_key(&ChunkId::new(10)));
        assert!(state.cards.contains_key(&CardId::new(20)));
        assert!(
            state
                .artifacts
                .get(&ArtifactId::new(1))
                .is_some_and(|artifact| artifact.chunk_ids.contains(&ChunkId::new(10))
                    && artifact.card_ids.contains(&CardId::new(20)))
        );
        assert!(output.events.iter().any(|event| matches!(
            event.event,
            DomainEvent::CardCreated {
                card_id: CardId(20),
                artifact_id: ArtifactId(1),
            }
        )));
        Ok(())
    }

    #[test]
    fn task_status_transition_is_restricted() -> Result<(), DomainError> {
        let mut state = KernelState::new();
        state.apply_input(DomainInput::OpenTask(OpenTaskInput {
            task_id: TaskId::new(3),
            title: "initial".to_string(),
            priority: TaskPriority::Normal,
            artifact_id: None,
        }))?;

        assert!(
            state
                .apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                    task_id: TaskId::new(3),
                    to: TaskStatus::Active,
                }))
                .is_err()
        );

        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(3),
            to: TaskStatus::Open,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(3),
            to: TaskStatus::Active,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(3),
            to: TaskStatus::Validating,
        }))?;
        assert!(matches!(
            state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id: TaskId::new(3),
                to: TaskStatus::CompletedVerified,
            })),
            Err(DomainError::ValidationRequired { .. })
        ));
        state.apply_input(DomainInput::RecordValidationReport(
            RecordValidationReportInput {
                report_id: ValidationReportId::new(9),
                task_id: Some(TaskId::new(3)),
                passed: true,
                warnings: Vec::new(),
            },
        ))?;
        state.apply_input(DomainInput::CompleteTask(CompleteTaskInput {
            task_id: TaskId::new(3),
            validation_report_id: ValidationReportId::new(9),
        }))?;

        let task = state
            .tasks
            .get(&TaskId::new(3))
            .ok_or(DomainError::MissingTask { id: TaskId::new(3) })?;
        assert_eq!(task.status, TaskStatus::CompletedVerified);
        assert_eq!(task.validation_report_id, Some(ValidationReportId::new(9)));
        Ok(())
    }

    #[test]
    fn validated_completion_is_the_only_completion_path() -> Result<(), DomainError> {
        let mut state = KernelState::new();
        state.apply_input(DomainInput::OpenTask(OpenTaskInput {
            task_id: TaskId::new(7),
            title: "Ship the verified answer".to_string(),
            priority: TaskPriority::Normal,
            artifact_id: None,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(7),
            to: TaskStatus::Open,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(7),
            to: TaskStatus::Active,
        }))?;
        state.apply_input(DomainInput::RecordValidationReport(
            RecordValidationReportInput {
                report_id: ValidationReportId::new(80),
                task_id: Some(TaskId::new(7)),
                passed: false,
                warnings: vec!["non-blocking warning".to_string()],
            },
        ))?;

        assert_eq!(
            state
                .apply_input(DomainInput::CompleteTask(CompleteTaskInput {
                    task_id: TaskId::new(7),
                    validation_report_id: ValidationReportId::new(80),
                }))
                .err(),
            Some(DomainError::InvalidTaskTransition {
                task_id: TaskId::new(7),
                from: TaskStatus::Active,
                to: TaskStatus::CompletedWithWarnings,
            })
        );

        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(7),
            to: TaskStatus::Validating,
        }))?;
        let output = state.apply_input(DomainInput::CompleteTask(CompleteTaskInput {
            task_id: TaskId::new(7),
            validation_report_id: ValidationReportId::new(80),
        }))?;

        let task = state
            .tasks
            .get(&TaskId::new(7))
            .ok_or(DomainError::MissingTask { id: TaskId::new(7) })?;
        assert_eq!(task.status, TaskStatus::CompletedWithWarnings);
        assert_eq!(task.validation_report_id, Some(ValidationReportId::new(80)));
        assert!(matches!(
            output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::TaskCompletionRecorded {
                    task_id,
                    status,
                    validation_report_id,
                },
                ..
            }] if *task_id == TaskId::new(7)
                && *status == TaskStatus::CompletedWithWarnings
                && *validation_report_id == ValidationReportId::new(80)
        ));
        assert_eq!(
            output.effects,
            vec![
                MaestriaEffect::PersistEvent {
                    event: output.events[0].event.clone(),
                },
                MaestriaEffect::PersistState(PersistStateRequest {
                    reason: "validated task completion".to_string(),
                }),
            ]
        );
        Ok(())
    }

    #[test]
    fn complete_task_requires_validation_report() -> Result<(), DomainError> {
        let mut state = KernelState::new();
        state.apply_input(DomainInput::OpenTask(OpenTaskInput {
            task_id: TaskId::new(7),
            title: "Ship the verified answer".to_string(),
            priority: TaskPriority::Normal,
            artifact_id: None,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(7),
            to: TaskStatus::Open,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(7),
            to: TaskStatus::Active,
        }))?;
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(7),
            to: TaskStatus::Validating,
        }))?;

        assert_eq!(
            state
                .apply_input(DomainInput::CompleteTask(CompleteTaskInput {
                    task_id: TaskId::new(7),
                    validation_report_id: ValidationReportId::new(80),
                }))
                .err(),
            Some(DomainError::MissingValidationReport {
                id: ValidationReportId::new(80)
            })
        );
        Ok(())
    }

    #[test]
    fn evidence_kind_preserves_provenance_and_triggers_claim_validation() -> Result<(), DomainError>
    {
        let mut state = KernelState::new();
        register_artifact_and_claim(&mut state)?;
        let kind = EvidenceKind::CommandOutput {
            harness_run: HarnessRunId::new(77),
            stream: OutputStream::Stderr,
            blob: BlobId::new(55),
        };

        let output = state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: kind.clone(),
            excerpt: "stderr: assertion failed".to_string(),
            observed_at: LogicalTick::new(12),
        }))?;

        assert!(matches!(
            output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::EvidenceRecorded {
                    evidence_id,
                    artifact_id,
                    claim_id,
                    kind: event_kind,
                },
                ..
            }] if *evidence_id == EvidenceId::new(40)
                && *artifact_id == ArtifactId::new(1)
                && *claim_id == Some(ClaimId::new(20))
                && *event_kind == kind
        ));
        assert_eq!(
            output.effects,
            vec![
                MaestriaEffect::PersistEvent {
                    event: output.events[0].event.clone(),
                },
                MaestriaEffect::RunValidation(RunValidationRequest {
                    task_id: None,
                    claim_id: Some(ClaimId::new(20)),
                    validation_report_id: ValidationReportId::new(0),
                }),
            ]
        );

        let evidence =
            state
                .evidences
                .get(&EvidenceId::new(40))
                .ok_or(DomainError::MissingEvidence {
                    id: EvidenceId::new(40),
                })?;
        assert_eq!(evidence.kind, kind);
        assert_eq!(evidence.excerpt, "stderr: assertion failed");
        assert_eq!(evidence.observed_at, LogicalTick::new(12));
        assert_eq!(
            state
                .claims
                .get(&ClaimId::new(20))
                .ok_or(DomainError::MissingClaim {
                    id: ClaimId::new(20)
                })?
                .evidence_ids,
            BTreeSet::from([EvidenceId::new(40)])
        );
        Ok(())
    }

    #[test]
    fn relation_and_memory_candidates_are_domain_owned_and_evidence_bound()
    -> Result<(), DomainError> {
        let mut state = KernelState::new();
        register_artifact_and_claim(&mut state)?;
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: file_span_kind(),
            excerpt: "first chunk".to_string(),
            observed_at: LogicalTick::new(12),
        }))?;

        assert_eq!(
            state
                .apply_input(DomainInput::CreateRelation(CreateRelationInput {
                    relation_id: RelationId::new(99),
                    source: RelationEndpoint::Claim(ClaimId::new(20)),
                    kind: RelationKind::Supports,
                    target: RelationEndpoint::Artifact(ArtifactId::new(1)),
                    evidence_id: Some(EvidenceId::new(404)),
                    confidence_milli: 875,
                }))
                .err(),
            Some(DomainError::MissingEvidence {
                id: EvidenceId::new(404)
            })
        );

        let relation_output =
            state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
                relation_id: RelationId::new(70),
                source: RelationEndpoint::Claim(ClaimId::new(20)),
                kind: RelationKind::Supports,
                target: RelationEndpoint::Artifact(ArtifactId::new(1)),
                evidence_id: Some(EvidenceId::new(40)),
                confidence_milli: 875,
            }))?;
        assert_eq!(
            state.relations.get(&RelationId::new(70)),
            Some(&Relation {
                id: RelationId::new(70),
                source: RelationEndpoint::Claim(ClaimId::new(20)),
                kind: RelationKind::Supports,
                target: RelationEndpoint::Artifact(ArtifactId::new(1)),
                evidence_id: Some(EvidenceId::new(40)),
                confidence_milli: 875,
            })
        );
        assert_eq!(
            relation_output.effects,
            vec![
                MaestriaEffect::PersistEvent {
                    event: relation_output.events[0].event.clone(),
                },
                MaestriaEffect::UpdateGraph(UpdateGraphRequest {
                    relation_id: RelationId::new(70),
                }),
            ]
        );

        assert!(matches!(
            state.apply_input(DomainInput::CreateMemoryCandidate(
                CreateMemoryCandidateInput {
                    candidate_id: MemoryCandidateId::new(91),
                    claim_id: ClaimId::new(20),
                    evidence_ids: Vec::new(),
                    confidence_milli: 720,
                },
            )),
            Err(DomainError::EvidenceRequired {
                kind: "memory_candidate",
                id: 91,
            })
        ));

        let candidate_output = state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(90),
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40), EvidenceId::new(40)],
                confidence_milli: 720,
            },
        ))?;
        assert!(matches!(
            candidate_output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::MemoryCandidateCreated {
                    candidate_id,
                    claim_id,
                    ..
                },
                ..
            }] if *candidate_id == MemoryCandidateId::new(90)
                && *claim_id == ClaimId::new(20)
        ));
        let candidate = state
            .memory_candidates
            .get(&MemoryCandidateId::new(90))
            .ok_or(DomainError::MissingMemoryCandidate {
                id: MemoryCandidateId::new(90),
            })?;
        assert!(candidate.has_evidence());
        assert_eq!(candidate.claim_id, ClaimId::new(20));
        assert_eq!(
            candidate.evidence_ids,
            BTreeSet::from([EvidenceId::new(40)])
        );
        assert_eq!(candidate.confidence_milli, 720);
        Ok(())
    }

    #[test]
    fn promote_memory_creates_active_memory_from_candidate() -> Result<(), DomainError> {
        let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;

        let output = state.apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
            memory_id: MemoryId::new(100),
            candidate_id: MemoryCandidateId::new(90),
        }))?;

        let memory = state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?;
        assert_eq!(memory.candidate_id, MemoryCandidateId::new(90));
        assert_eq!(memory.claim_id, ClaimId::new(20));
        assert_eq!(memory.evidence_ids, BTreeSet::from([EvidenceId::new(40)]));
        assert_eq!(memory.status, MemoryStatus::Active);
        assert!(matches!(
            output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::MemoryPromoted {
                    memory_id,
                    candidate_id,
                },
                ..
            }] if *memory_id == MemoryId::new(100)
                && *candidate_id == MemoryCandidateId::new(90)
        ));
        assert_eq!(
            output.effects,
            vec![MaestriaEffect::PersistEvent {
                event: output.events[0].event.clone(),
            }]
        );
        Ok(())
    }

    #[test]
    fn promote_memory_rejects_missing_candidate() -> Result<(), DomainError> {
        let mut state = KernelState::new();

        assert_eq!(
            state
                .apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
                    memory_id: MemoryId::new(100),
                    candidate_id: MemoryCandidateId::new(404),
                }))
                .err(),
            Some(DomainError::MissingMemoryCandidate {
                id: MemoryCandidateId::new(404),
            })
        );
        Ok(())
    }

    #[test]
    fn contradict_memory_marks_memory_contradicted() -> Result<(), DomainError> {
        let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
        state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(91),
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40)],
                confidence_milli: 650,
            },
        ))?;
        promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

        let output = state.apply_input(DomainInput::ContradictMemory(ContradictMemoryInput {
            memory_id: MemoryId::new(100),
            contradicting_candidate_id: MemoryCandidateId::new(91),
        }))?;

        assert_eq!(
            state
                .memories
                .get(&MemoryId::new(100))
                .ok_or(DomainError::MissingMemory {
                    id: MemoryId::new(100),
                })?
                .status,
            MemoryStatus::Contradicted
        );
        assert!(matches!(
            output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::MemoryContradicted {
                    memory_id,
                    contradicting_candidate_id,
                },
                ..
            }] if *memory_id == MemoryId::new(100)
                && *contradicting_candidate_id == MemoryCandidateId::new(91)
        ));
        Ok(())
    }

    #[test]
    fn deprecate_memory_marks_memory_deprecated() -> Result<(), DomainError> {
        let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
        promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

        let output = state.apply_input(DomainInput::DeprecateMemory(DeprecateMemoryInput {
            memory_id: MemoryId::new(100),
        }))?;

        assert_eq!(
            state
                .memories
                .get(&MemoryId::new(100))
                .ok_or(DomainError::MissingMemory {
                    id: MemoryId::new(100),
                })?
                .status,
            MemoryStatus::Deprecated
        );
        assert!(matches!(
            output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::MemoryDeprecated { memory_id },
                ..
            }] if *memory_id == MemoryId::new(100)
        ));
        Ok(())
    }

    #[test]
    fn supersede_memory_marks_memory_superseded() -> Result<(), DomainError> {
        let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
        state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(91),
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40)],
                confidence_milli: 650,
            },
        ))?;
        promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;
        promote_memory(&mut state, MemoryId::new(101), MemoryCandidateId::new(91))?;

        let output = state.apply_input(DomainInput::SupersedeMemory(SupersedeMemoryInput {
            memory_id: MemoryId::new(100),
            by_memory_id: MemoryId::new(101),
        }))?;

        assert_eq!(
            state
                .memories
                .get(&MemoryId::new(100))
                .ok_or(DomainError::MissingMemory {
                    id: MemoryId::new(100),
                })?
                .status,
            MemoryStatus::Superseded
        );
        assert!(matches!(
            output.events.as_slice(),
            [DomainEventEnvelope {
                event: DomainEvent::MemorySuperseded {
                    memory_id,
                    by_memory_id,
                },
                ..
            }] if *memory_id == MemoryId::new(100)
                && *by_memory_id == MemoryId::new(101)
        ));
        Ok(())
    }

    #[test]
    fn record_validation_report_emits_informational_event() -> Result<(), DomainError> {
        let mut state = KernelState::new();
        state.apply_input(DomainInput::OpenTask(OpenTaskInput {
            task_id: TaskId::new(50),
            title: "Validate answer".to_string(),
            priority: TaskPriority::Normal,
            artifact_id: None,
        }))?;

        let output = state.apply_input(DomainInput::RecordValidationReport(
            RecordValidationReportInput {
                report_id: ValidationReportId::new(80),
                task_id: Some(TaskId::new(50)),
                passed: true,
                warnings: vec!["minor style warning".to_string()],
            },
        ))?;

        assert_eq!(output.events.len(), 1);
        if let DomainEvent::ValidationReportCreated {
            report_id,
            task_id,
            passed,
            warnings,
        } = &output.events[0].event
        {
            assert_eq!(*report_id, ValidationReportId::new(80));
            assert_eq!(*task_id, Some(TaskId::new(50)));
            assert!(*passed);
            assert_eq!(warnings, &vec!["minor style warning".to_string()]);
        } else {
            panic!("expected validation report created event");
        }
        assert!(
            output
                .effects
                .iter()
                .any(|effect| matches!(effect, MaestriaEffect::PersistEvent { .. }))
        );
        Ok(())
    }

    #[test]
    fn replay_events_reconstructs_new_memory_event_state() -> Result<(), DomainError> {
        let inputs = vec![
            DomainInput::RegisterArtifact(RegisterArtifactInput {
                artifact_id: ArtifactId::new(1),
                title: "Project Notes".to_string(),
            }),
            DomainInput::CreateClaim(CreateClaimInput {
                claim_id: ClaimId::new(20),
                artifact_id: ArtifactId::new(1),
                text: "Claim from evidence".to_string(),
                evidence_ids: Vec::new(),
            }),
            DomainInput::RecordEvidence(RecordEvidenceInput {
                evidence_id: EvidenceId::new(40),
                artifact_id: ArtifactId::new(1),
                claim_id: Some(ClaimId::new(20)),
                kind: file_span_kind(),
                excerpt: "first chunk".to_string(),
                observed_at: LogicalTick::new(12),
            }),
            DomainInput::CreateMemoryCandidate(CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(90),
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40)],
                confidence_milli: 720,
            }),
            DomainInput::CreateMemoryCandidate(CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(91),
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40)],
                confidence_milli: 650,
            }),
            DomainInput::PromoteMemory(PromoteMemoryInput {
                memory_id: MemoryId::new(100),
                candidate_id: MemoryCandidateId::new(90),
            }),
            DomainInput::PromoteMemory(PromoteMemoryInput {
                memory_id: MemoryId::new(101),
                candidate_id: MemoryCandidateId::new(91),
            }),
            DomainInput::ContradictMemory(ContradictMemoryInput {
                memory_id: MemoryId::new(100),
                contradicting_candidate_id: MemoryCandidateId::new(91),
            }),
            DomainInput::DeprecateMemory(DeprecateMemoryInput {
                memory_id: MemoryId::new(101),
            }),
            DomainInput::SupersedeMemory(SupersedeMemoryInput {
                memory_id: MemoryId::new(100),
                by_memory_id: MemoryId::new(101),
            }),
            DomainInput::RecordValidationReport(RecordValidationReportInput {
                report_id: ValidationReportId::new(80),
                task_id: None,
                passed: false,
                warnings: Vec::new(),
            }),
        ];
        let (_state, events, _effects) = replay_inputs(&inputs)?;
        let replayed = replay_events(&events)?;

        assert_eq!(replayed.event_log, events);
        assert_eq!(
            replayed
                .memories
                .get(&MemoryId::new(100))
                .ok_or(DomainError::MissingMemory {
                    id: MemoryId::new(100),
                })?
                .status,
            MemoryStatus::Superseded
        );
        assert_eq!(
            replayed
                .memories
                .get(&MemoryId::new(101))
                .ok_or(DomainError::MissingMemory {
                    id: MemoryId::new(101),
                })?
                .status,
            MemoryStatus::Deprecated
        );
        assert!(events.iter().any(|event| matches!(
            event.event,
            DomainEvent::ValidationReportCreated {
                report_id: ValidationReportId(80),
                task_id: None,
                passed: false,
                ..
            }
        )));
        Ok(())
    }

    #[test]
    fn replay_keeps_new_event_and_effect_shapes_deterministic() -> Result<(), DomainError> {
        let inputs = vec![
            DomainInput::RegisterArtifact(RegisterArtifactInput {
                artifact_id: ArtifactId::new(1),
                title: "Project Notes".to_string(),
            }),
            DomainInput::CreateClaim(CreateClaimInput {
                claim_id: ClaimId::new(20),
                artifact_id: ArtifactId::new(1),
                text: "Claim from evidence".to_string(),
                evidence_ids: Vec::new(),
            }),
            DomainInput::RecordEvidence(RecordEvidenceInput {
                evidence_id: EvidenceId::new(40),
                artifact_id: ArtifactId::new(1),
                claim_id: Some(ClaimId::new(20)),
                kind: file_span_kind(),
                excerpt: "first chunk".to_string(),
                observed_at: LogicalTick::new(12),
            }),
            DomainInput::OpenTask(OpenTaskInput {
                task_id: TaskId::new(50),
                title: "Summarize artifact".to_string(),
                priority: TaskPriority::Normal,
                artifact_id: Some(ArtifactId::new(1)),
            }),
            DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id: TaskId::new(50),
                to: TaskStatus::Open,
            }),
            DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id: TaskId::new(50),
                to: TaskStatus::Active,
            }),
            DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id: TaskId::new(50),
                to: TaskStatus::Validating,
            }),
            DomainInput::RecordValidationReport(RecordValidationReportInput {
                report_id: ValidationReportId::new(80),
                task_id: Some(TaskId::new(50)),
                passed: true,
                warnings: Vec::new(),
            }),
            DomainInput::CompleteTask(CompleteTaskInput {
                task_id: TaskId::new(50),
                validation_report_id: ValidationReportId::new(80),
            }),
            DomainInput::CreateRelation(CreateRelationInput {
                relation_id: RelationId::new(70),
                source: RelationEndpoint::Claim(ClaimId::new(20)),
                kind: RelationKind::Supports,
                target: RelationEndpoint::Task(TaskId::new(50)),
                evidence_id: Some(EvidenceId::new(40)),
                confidence_milli: 875,
            }),
            DomainInput::CreateMemoryCandidate(CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(90),
                claim_id: ClaimId::new(20),
                evidence_ids: vec![EvidenceId::new(40)],
                confidence_milli: 720,
            }),
        ];

        let (state_a, events_a, effects_a) = replay_inputs(&inputs)?;
        let (state_b, events_b, effects_b) = replay_inputs(&inputs)?;

        assert_eq!(state_a, state_b);
        assert_eq!(events_a, events_b);
        assert_eq!(effects_a, effects_b);
        assert!(events_a.iter().any(|envelope| matches!(
            &envelope.event,
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } if *task_id == TaskId::new(50)
                && *status == TaskStatus::CompletedVerified
                && *validation_report_id == ValidationReportId::new(80)
        )));
        assert!(events_a.iter().any(|envelope| matches!(
            &envelope.event,
            DomainEvent::RelationCreated { relation_id } if *relation_id == RelationId::new(70)
        )));
        assert!(events_a.iter().any(|envelope| matches!(
            &envelope.event,
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                ..
            } if *candidate_id == MemoryCandidateId::new(90)
                && *claim_id == ClaimId::new(20)
        )));
        assert!(effects_a.iter().any(|effect| matches!(
            effect,
            MaestriaEffect::PersistState(PersistStateRequest { reason })
                if reason == "validated task completion"
        )));
        assert!(effects_a.iter().any(|effect| matches!(
            effect,
            MaestriaEffect::UpdateGraph(UpdateGraphRequest { relation_id })
                if *relation_id == RelationId::new(70)
        )));
        Ok(())
    }

    #[test]
    fn replay_is_deterministic() -> Result<(), DomainError> {
        let (state_a, events_a, effects_a) = run_replay_once()?;
        let (state_b, events_b, effects_b) = run_replay_once()?;

        assert_eq!(state_a, state_b);
        assert_eq!(events_a, events_b);
        assert_eq!(effects_a, effects_b);
        Ok(())
    }

    #[test]
    fn replay_events_are_equivalent() -> Result<(), DomainError> {
        let (state, events, _) = run_replay_once()?;
        let replayed = replay_events(&events)?;
        assert_eq!(state.event_log, replayed.event_log);
        assert_eq!(state.artifacts.len(), replayed.artifacts.len());
        assert_eq!(state.claims.len(), replayed.claims.len());
        Ok(())
    }

    #[test]
    fn kernel_does_not_depend_on_forbidden_runtime_crates_or_operators() {
        let source = include_str!("lib.rs");
        let prelude = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(head, _)| head);
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));

        for forbidden in ["tokio", "sqlx", "reqwest", "tantivy", "axum"] {
            assert!(
                !manifest.contains(&format!("{forbidden} =")),
                "found forbidden runtime dependency token: {forbidden}"
            );
            assert!(
                !prelude.contains(forbidden),
                "found forbidden runtime token in source: {forbidden}"
            );
        }

        for forbidden in ["unwrap(", "expect(", "panic!("] {
            assert!(
                !prelude.contains(forbidden),
                "found forbidden failure path token: {forbidden}"
            );
        }
    }
}
