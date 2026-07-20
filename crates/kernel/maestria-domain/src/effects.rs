use crate::events::DomainEventEnvelope;
use crate::ids::{
    ApprovalId, ArtifactId, BlobId, ChunkId, ClaimId, HarnessRunId, RelationId, ScopeId, TaskId,
    ValidationReportId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistStateRequest {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseArtifactRequest {
    pub artifact_id: ArtifactId,
    pub source_path: String,
    pub source_bytes: Vec<u8>,
    pub source_blob: Option<BlobId>,
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
    pub max_bytes: usize,
    pub max_requests: u32,
    pub max_latency_ms: u32,
    pub allowed_domains: Vec<String>,
    pub allowed_content_types: Vec<String>,
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
pub struct SearchKnowledgeRequest {
    pub task_id: Option<TaskId>,
    pub plan: crate::search::SearchPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaestriaEffect {
    PersistEvent { envelope: Box<DomainEventEnvelope> },
    PersistState(PersistStateRequest),
    ParseArtifact(ParseArtifactRequest),
    IndexFullText(IndexFullTextRequest),
    IndexVector(IndexVectorRequest),
    UpdateGraph(UpdateGraphRequest),
    QueryHarness(QueryHarnessRequest),
    FetchWeb(FetchWebRequest),
    RunValidation(RunValidationRequest),
    RequestApproval(RequestApprovalRequest),
    EmitDiagnostic(DiagnosticEvent),
    SearchKnowledge(Box<SearchKnowledgeRequest>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KernelOutput {
    pub events: Vec<DomainEventEnvelope>,
    pub effects: Vec<MaestriaEffect>,
}
