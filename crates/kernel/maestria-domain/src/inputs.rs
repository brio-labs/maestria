use crate::entities::{EvidenceKind, RelationEndpoint, RelationKind, TaskPriority, TaskStatus};
use crate::ids::{
    ApprovalId, ArtifactId, BlobId, CardId, ChunkId, ClaimId, EvidenceId, HarnessRunId,
    IndexGenerationId, LogicalTick, MemoryCandidateId, MemoryId, RelationId, ScopeId, TaskId,
    ValidationReportId,
};

use crate::security::SecurityMetadata;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterArtifactInput {
    pub artifact_id: ArtifactId,
    pub title: String,
    pub security: Option<SecurityMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterChunkInput {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub node_id: crate::types::StructureNodeId,
    pub source_span: crate::provenance::SourceSpan,
    pub representations: Vec<crate::provenance::ParsedRepresentation>,
    pub order: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCardInput {
    pub card_id: CardId,
    pub artifact_id: ArtifactId,
    pub node_id: crate::types::StructureNodeId,
    pub source_span: crate::provenance::SourceSpan,
    pub title: String,
    pub body: String,
    pub security: Option<SecurityMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordEvidenceInput {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub kind: EvidenceKind,
    pub excerpt: String,
    pub observed_at: LogicalTick,
    pub security: Option<SecurityMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateClaimInput {
    pub claim_id: ClaimId,
    pub artifact_id: ArtifactId,
    pub text: String,
    pub evidence_ids: Vec<EvidenceId>,
    pub security: Option<SecurityMetadata>,
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
    pub security: Option<SecurityMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateMemoryCandidateInput {
    pub candidate_id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: Vec<EvidenceId>,
    pub confidence_milli: u16,
    pub security: Option<SecurityMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposeMemoryCandidateInput {
    pub claim_id: ClaimId,
    pub candidate_id: MemoryCandidateId,
    pub text: String,
    pub evidence_ids: Vec<EvidenceId>,
    pub confidence_milli: u16,
    pub security: Option<SecurityMetadata>,
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
pub struct SourceRemoved {
    pub artifact_id: ArtifactId,
    pub source_path: String,
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
    pub artifact_version_id: crate::ids::ArtifactVersionId,
    pub content_hash: crate::search::ContentHash,
    pub status: crate::provenance::ParseStatus,
    pub tree_root_id: Option<crate::ids::StructureNodeId>,
    pub tree_nodes: Vec<crate::search::StructureNode>,
    pub chunks: Vec<RegisterChunkInput>,
    pub cards: Vec<CreateCardInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrRequested {
    pub intent: crate::ocr::OcrIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrCompleted {
    pub artifact_id: ArtifactId,
    pub completion: crate::ocr::OcrCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrFailed {
    pub artifact_id: ArtifactId,
    pub request_id: crate::ocr::OcrRequestId,
    pub reason: String,
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
    pub run_id: HarnessRunId,
    pub generation: u64,
    pub task_id: Option<TaskId>,
    pub command: String,
    pub exit_code: i32,
    pub output: String,
}

/// Request execution of a harness command. The runtime owns governance,
/// effect-journal admission, and adapter execution for this request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRunRequested {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub generation: Option<u64>,
    pub capability: String,
    pub scope_id: ScopeId,
    pub approval_id: Option<ApprovalId>,
    pub command: String,
}
/// A fully validated model-agent proposal crossing the canonical runtime boundary.
///
/// The runtime and effect executor carry this value unchanged so approval
/// continuations can resume the exact request rather than reconstructing it
/// from a command string.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentProposalRequest {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub query: String,
    pub limit: usize,
    pub evidence_ids: Vec<EvidenceId>,
    pub capability: String,
    pub command: String,
    pub working_directory: String,
    pub timeout_secs: u64,
    pub expected_generation: u64,
    pub task_validation: bool,
    pub memory_candidate: bool,
    pub approval_id: Option<ApprovalId>,
    pub journal_generation: Option<u64>,
    pub correlation_id: u64,
}

impl ModelAgentProposalRequest {
    pub fn into_harness_request(self) -> crate::effects::QueryHarnessProposalRequest {
        crate::effects::QueryHarnessProposalRequest { proposal: self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentSearchResult {
    pub trace_id: u64,
    pub evidence_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentHarnessResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentValidationResult {
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelAgentMemoryDecision {
    Promote,
    RequireEvidence,
    RequireReview,
    Deny,
}

impl ModelAgentMemoryDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::RequireEvidence => "require_evidence",
            Self::RequireReview => "require_review",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentMemoryResult {
    pub candidate_id: MemoryCandidateId,
    pub confidence_milli: u16,
    pub decision: ModelAgentMemoryDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelAgentTerminalStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentProposalResult {
    pub run_id: HarnessRunId,
    pub correlation_id: u64,
    pub status: ModelAgentTerminalStatus,
    pub search: Option<ModelAgentSearchResult>,
    pub harness: Option<ModelAgentHarnessResult>,
    pub validation: Option<ModelAgentValidationResult>,
    pub memory_candidate: Option<ModelAgentMemoryResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCompleted {
    pub claim_id: ClaimId,
    pub valid: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestTaskValidation {
    pub task_id: TaskId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecision {
    pub approval_id: ApprovalId,
    pub task_id: Option<TaskId>,
    pub approved: bool,
    pub affects_task: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchExecutedInput {
    pub query: String,
    pub limit: usize,
    pub evidence_ids: Vec<EvidenceId>,
    pub pack_metadata: Option<Box<crate::evidence_pack::EvidencePackMetadataRecord>>,
    pub at: LogicalTick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchKnowledgeRequested {
    pub task_id: Option<TaskId>,
    pub plan: crate::search::SearchPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchKnowledgeCompleted {
    pub task_id: Option<TaskId>,
    pub plan: Box<crate::search::SearchPlan>,
    pub outcome: crate::search::SearchOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchWebRequested {
    pub request: crate::effects::FetchWebRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartIndexGenerationInput {
    pub id: IndexGenerationId,
    pub name: crate::generations::RepresentationName,
    pub corpus_snapshot: crate::ids::CorpusSnapshotId,
    pub fingerprint: crate::generations::IndexFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionIndexGenerationInput {
    pub id: IndexGenerationId,
    pub to: crate::generations::IndexLifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainInput {
    ModelAgentProposalRequested(ModelAgentProposalRequest),
    /// Resume a previously recorded proposal without creating a duplicate run request event.
    ModelAgentProposalResumed(ModelAgentProposalRequest),
    RegisterArtifact(RegisterArtifactInput),
    RegisterChunk(RegisterChunkInput),
    ModelAgentProposalCompleted(ModelAgentProposalResult),
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
    RequestTaskValidation(RequestTaskValidation),
    UserIntent(UserIntent),
    FullTextIndexCompleted(FullTextIndexCompleted),
    StartFullTextIndex(StartFullTextIndex),
    ArtifactDetected(ArtifactDetected),
    SourceRemoved(SourceRemoved),
    ParserCompleted(ParserResult),
    OcrRequested(OcrRequested),
    OcrCompleted(OcrCompleted),
    OcrFailed(OcrFailed),
    ParserStarted(ParserStarted),
    ResumeParser(ParserStarted),
    SearchCompleted(SearchResultSet),
    HarnessRunRequested(HarnessRunRequested),
    HarnessRunCompleted(HarnessRunCompleted),
    ValidationCompleted(ValidationCompleted),
    ApprovalResolved(ApprovalDecision),
    FetchWebRequested(FetchWebRequested),
    StartIndexGeneration(StartIndexGenerationInput),
    SearchKnowledgeRequested(SearchKnowledgeRequested),
    SearchExecuted(SearchExecutedInput),
    SearchKnowledgeCompleted(SearchKnowledgeCompleted),
    TransitionIndexGeneration(TransitionIndexGenerationInput),
    ClockTick(LogicalTick),
}
