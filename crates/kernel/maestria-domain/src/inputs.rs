use crate::entities::{EvidenceKind, RelationEndpoint, RelationKind, TaskPriority, TaskStatus};
use crate::ids::{
    ApprovalId, ArtifactId, BlobId, CardId, ChunkId, ClaimId, EvidenceId, LogicalTick,
    MemoryCandidateId, MemoryId, RelationId, TaskId, ValidationReportId,
};

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
pub struct LinkEvidenceToTaskInput {
    pub task_id: TaskId,
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
pub struct ProposeMemoryCandidateInput {
    pub claim_id: ClaimId,
    pub candidate_id: MemoryCandidateId,
    pub text: String,
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
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserStarted {
    pub artifact_id: ArtifactId,
    pub title: String,
    pub source_path: String,
    pub content_hash: String,
    pub blob_id: BlobId,
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
pub struct FullTextIndexCompleted {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartFullTextIndex {
    pub artifact_id: ArtifactId,
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
    pub approval_id: ApprovalId,
    pub task_id: TaskId,
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchExecutedInput {
    pub query: String,
    pub limit: usize,
    pub evidence_ids: Vec<EvidenceId>,
    pub at: LogicalTick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainInput {
    RegisterArtifact(RegisterArtifactInput),
    RegisterChunk(RegisterChunkInput),
    CreateCard(CreateCardInput),
    RecordEvidence(RecordEvidenceInput),
    CreateClaim(CreateClaimInput),
    ProposeMemoryCandidate(ProposeMemoryCandidateInput),
    OpenTask(OpenTaskInput),
    ChangeTaskStatus(ChangeTaskStatusInput),
    CompleteTask(CompleteTaskInput),
    LinkEvidenceToTask(LinkEvidenceToTaskInput),
    LinkEvidenceToClaim(LinkEvidenceToClaimInput),
    CreateRelation(CreateRelationInput),
    CreateMemoryCandidate(CreateMemoryCandidateInput),
    PromoteMemory(PromoteMemoryInput),
    ContradictMemory(ContradictMemoryInput),
    DeprecateMemory(DeprecateMemoryInput),
    SupersedeMemory(SupersedeMemoryInput),
    RecordValidationReport(RecordValidationReportInput),

    UserIntent(UserIntent),
    FullTextIndexCompleted(FullTextIndexCompleted),
    StartFullTextIndex(StartFullTextIndex),
    ArtifactDetected(ArtifactDetected),
    ParserStarted(ParserStarted),
    ResumeParser(ParserStarted),
    ParserCompleted(ParserResult),
    SearchCompleted(SearchResultSet),
    HarnessRunCompleted(HarnessRunCompleted),
    ValidationCompleted(ValidationCompleted),
    ApprovalResolved(ApprovalDecision),
    SearchExecuted(SearchExecutedInput),
    ClockTick(LogicalTick),
}
