use crate::entities::{RelationEndpoint, RelationKind, TaskPriority};
use crate::evidence_source::EvidenceKind;
use crate::ids::{
    ApprovalId, ArtifactId, BlobId, CardId, ChunkId, ClaimId, EvidenceId, HarnessRunId,
    IndexGenerationId, LogicalTick, MemoryCandidateId, MemoryId, RelationId, ScopeId, TaskId,
    ValidationReportId,
};
use crate::model_agent::{ModelAgentProposalRequest, ModelAgentProposalResult};
use crate::notebook_inputs::*;
use crate::task_status::TaskStatus;

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
    pub content_hash: crate::search::ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRemoved {
    pub artifact_id: ArtifactId,
    pub source_path: String,
    pub content_hash: crate::search::ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserStarted {
    pub artifact_id: ArtifactId,
    pub title: String,
    pub source_path: String,
    pub content_hash: crate::search::ContentHash,
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
    pub execution: crate::effects::HarnessExecution,
    pub capability: String,
    pub scope_id: ScopeId,
    pub command: String,
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

/// A resolved approval decision at the domain boundary.
///
/// The decision either records an acknowledgement that affects no task or
/// approves/denies a resolution that transitions the referenced task. Only
/// the resolution form requires and transitions a task; the acknowledgement
/// form may still carry the task for audit purposes (model-agent approvals),
/// so the pair is an enum instead of correlated flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Record the decision without transitioning any task.
    Acknowledge {
        approval_id: ApprovalId,
        task_id: Option<TaskId>,
        approved: bool,
    },
    /// Approve or deny the resolution, transitioning the referenced task.
    Resolve {
        approval_id: ApprovalId,
        task_id: TaskId,
        approved: bool,
    },
}

impl ApprovalDecision {
    pub fn approval_id(&self) -> ApprovalId {
        match self {
            Self::Acknowledge { approval_id, .. } | Self::Resolve { approval_id, .. } => {
                *approval_id
            }
        }
    }

    pub fn task_id(&self) -> Option<TaskId> {
        match self {
            Self::Acknowledge { task_id, .. } => *task_id,
            Self::Resolve { task_id, .. } => Some(*task_id),
        }
    }

    pub fn approved(&self) -> bool {
        match self {
            Self::Acknowledge { approved, .. } | Self::Resolve { approved, .. } => *approved,
        }
    }
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

/// Issues a provider-owned realm read grant. It intentionally contains only
/// the credential digest, never the bearer credential bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRealmReadGrantInput {
    pub grant: crate::entities::RealmReadGrant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeRealmReadGrantInput {
    pub token_digest: crate::GrantTokenDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFederatedAccessInput {
    pub token_digest: crate::GrantTokenDigest,
    pub provider_realm: crate::RealmId,
    pub consumer_realm: crate::RealmId,
    pub record: crate::entities::FederatedAccessRecord,
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
    LinkEvidenceToClaim(LinkEvidenceToClaimInput),
    LinkEvidenceToTask(LinkEvidenceToTaskInput),
    CreateRelation(CreateRelationInput),
    CreateNotebook(CreateNotebookInput),
    RenameNotebook(RenameNotebookInput),
    DeleteNotebook(DeleteNotebookInput),
    AttachNotebookSource(AttachNotebookSourceInput),
    DetachNotebookSource(DetachNotebookSourceInput),
    SaveNotebookDraftRequested(SaveNotebookDraftRequested),
    NotebookDraftBlobStored(NotebookDraftBlobStored),
    NotebookDraftBlobStoreFailed(NotebookDraftBlobStoreFailed),
    DeleteNotebookDraft(DeleteNotebookDraftInput),
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
    IssueRealmReadGrant(IssueRealmReadGrantInput),
    RevokeRealmReadGrant(RevokeRealmReadGrantInput),
    RecordFederatedAccess(RecordFederatedAccessInput),
    SearchKnowledgeRequested(SearchKnowledgeRequested),
    SearchExecuted(SearchExecutedInput),
    SearchKnowledgeCompleted(SearchKnowledgeCompleted),
    TransitionIndexGeneration(TransitionIndexGenerationInput),
    ClockTick(LogicalTick),
}
