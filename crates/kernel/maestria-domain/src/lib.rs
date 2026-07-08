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
pub struct Evidence {
    pub id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub source: String,
    pub snippet: String,
    pub range: Option<ContentRange>,
    pub snapshot: Option<SnapshotId>,
    pub observed_at: LogicalTick,
}

impl Evidence {
    fn new(
        id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        source: String,
        snippet: String,
        observed_at: LogicalTick,
    ) -> Self {
        Self {
            id,
            artifact_id,
            claim_id,
            source,
            snippet,
            range: None,
            snapshot: None,
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
    Blocked,
    Completed,
    Cancelled,
}

impl TaskStatus {
    pub fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Draft => matches!(next, Self::Open | Self::Cancelled),
            Self::Open => matches!(next, Self::Active | Self::Cancelled),
            Self::Active => matches!(next, Self::Blocked | Self::Completed | Self::Cancelled),
            Self::Blocked => matches!(next, Self::Active | Self::Cancelled),
            Self::Completed | Self::Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub priority: TaskPriority,
    pub status: TaskStatus,
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
    ClaimValidationUpdated {
        claim_id: ClaimId,
        status: ClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: ClaimId,
        evidence_id: EvidenceId,
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
    TickObserved {
        at: LogicalTick,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreBlobRequest {
    pub artifact_id: ArtifactId,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFullTextRequest {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedChunksRequest {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryHarnessRequest {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunValidationRequest {
    pub claim_id: ClaimId,
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
    StoreBlob(StoreBlobRequest),
    IndexFullText(IndexFullTextRequest),
    EmbedChunks(EmbedChunksRequest),
    QueryHarness(QueryHarnessRequest),
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
    pub source: String,
    pub snippet: String,
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
pub struct LinkEvidenceToClaimInput {
    pub claim_id: ClaimId,
    pub evidence_id: EvidenceId,
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
    LinkEvidenceToClaim(LinkEvidenceToClaimInput),

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
    InvalidTaskTransition {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
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
            Self::InvalidTaskTransition { task_id, from, to } => {
                write!(f, "invalid task transition {task_id}: {from:?} -> {to:?}")
            }
            Self::EmptyIntent => write!(f, "user intent must not be empty"),
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
    pub tasks: BTreeMap<TaskId, Task>,
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
                        .push(MaestriaEffect::EmbedChunks(EmbedChunksRequest {
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
                            claim_id,
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
                        claim_id: input.claim_id,
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
            DomainInput::LinkEvidenceToClaim(input) => {
                let event = self.handle_link_evidence_to_claim(input.clone())?;
                output.events.push(event.clone());
                output
                    .effects
                    .push(MaestriaEffect::PersistEvent { event: event.event });
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
        if let Some(claim_id) = input.claim_id {
            if !self.claims.contains_key(&claim_id) {
                return Err(DomainError::MissingClaim { id: claim_id });
            }
        }

        self.evidences.insert(
            input.evidence_id,
            Evidence::new(
                input.evidence_id,
                input.artifact_id,
                input.claim_id,
                input.source,
                input.snippet,
                input.observed_at,
            ),
        );

        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.evidence_ids.insert(input.evidence_id);
        }
        if let Some(claim_id) = input.claim_id {
            if let Some(claim) = self.claims.get_mut(&claim_id) {
                claim.evidence_ids.insert(input.evidence_id);
            }
        }

        Ok(self.emit_event(DomainEvent::EvidenceRecorded {
            evidence_id: input.evidence_id,
            artifact_id: input.artifact_id,
            claim_id: input.claim_id,
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
        if let Some(artifact_id) = input.artifact_id {
            if !self.artifacts.contains_key(&artifact_id) {
                return Err(DomainError::MissingArtifact { id: artifact_id });
            }
        }

        let task = Task::new(input.task_id, input.title.clone(), input.priority);
        let artifact_id = input.artifact_id;
        self.tasks.insert(input.task_id, task);
        if let Some(artifact_id) = artifact_id {
            if let Some(task) = self.tasks.get_mut(&input.task_id) {
                task.artifact_ids.insert(artifact_id);
            }
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
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition { task_id, from, to });
        }
        task.status = to;
        Ok((from, to))
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

        let mut chunks = Vec::new();
        for chunk in input.chunks {
            chunks.push(self.handle_register_chunk(chunk)?);
        }

        let chunks_added = (chunks.len().min(u32::MAX as usize)) as u32;
        let parsed = self.emit_event(DomainEvent::ArtifactParsed {
            artifact_id: input.artifact_id,
            chunks_added,
        });
        chunks.push(parsed);
        Ok(chunks)
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

        if let Some(task_id) = task_id {
            if let Some(task) = self.tasks.get(&task_id) {
                if input.exit_code != 0 {
                    if task.status.can_transition_to(TaskStatus::Blocked) {
                        let (from, to) =
                            self.handle_change_task_status(task_id, TaskStatus::Blocked)?;
                        generated.push(self.emit_event(DomainEvent::TaskStatusChanged {
                            task_id,
                            from,
                            to,
                        }));
                    }
                } else if task.status == TaskStatus::Draft {
                    let (from, to) = self.handle_change_task_status(task_id, TaskStatus::Open)?;
                    generated.push(self.emit_event(DomainEvent::TaskStatusChanged {
                        task_id,
                        from,
                        to,
                    }));
                }
            }
        }

        if input.exit_code != 0 {
            if let Some(task_id) = task_id {
                generated.push(self.emit_event(DomainEvent::ApprovalRecorded {
                    task_id,
                    approved: false,
                }));
            }
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
                if let Some(claim_id) = claim_id {
                    if !self.claims.contains_key(claim_id) {
                        return Err(DomainError::MissingClaim { id: *claim_id });
                    }
                }

                self.evidences.insert(
                    *evidence_id,
                    Evidence::new(
                        *evidence_id,
                        *artifact_id,
                        *claim_id,
                        String::new(),
                        String::new(),
                        LogicalTick::new(0),
                    ),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.evidence_ids.insert(*evidence_id);
                }
                if let Some(claim_id) = claim_id {
                    if let Some(claim) = self.claims.get_mut(claim_id) {
                        claim.evidence_ids.insert(*evidence_id);
                    }
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
                source: "notes.txt:1-2".to_string(),
                snippet: "first chunk".to_string(),
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

    fn run_replay_once(
    ) -> Result<(KernelState, Vec<DomainEventEnvelope>, Vec<MaestriaEffect>), DomainError> {
        replay_inputs(&sample_inputs())
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

        assert!(state
            .apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id: TaskId::new(3),
                to: TaskStatus::Active,
            }))
            .is_err());

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
            to: TaskStatus::Completed,
        }))?;

        let final_status = state
            .tasks
            .get(&TaskId::new(3))
            .ok_or(DomainError::MissingTask { id: TaskId::new(3) })?
            .status;
        assert_eq!(final_status, TaskStatus::Completed);
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
