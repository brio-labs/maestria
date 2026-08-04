use crate::events::DomainEventEnvelope;
use crate::ids::{
    ApprovalId, ArtifactId, BlobId, ChunkId, ClaimId, HarnessRunId, RelationId, ScopeId, TaskId,
    ValidationReportId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseArtifactSource {
    Inline(Vec<u8>),
    Blob(BlobId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseArtifactRequest {
    pub artifact_id: ArtifactId,
    pub source_path: String,
    pub source: ParseArtifactSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrEffect {
    pub intent: crate::ocr::OcrIntent,
}

impl OcrEffect {
    pub fn new(intent: crate::ocr::OcrIntent) -> Self {
        Self { intent }
    }
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

/// How a harness run enters execution: fresh, resumed from a journal
/// generation, or continued after approval.
///
/// The correlated `generation`/`approval_id` option pair previously
/// admitted invalid combinations (an approval without a journal generation
/// or a generation without its continuation identity); the enum makes the
/// execution state space exhaustive (R56).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessExecution {
    Fresh,
    JournalRecovery {
        generation: u64,
    },
    ApprovalContinuation {
        approval_id: ApprovalId,
        generation: u64,
    },
}

impl HarnessExecution {
    pub fn generation(&self) -> Option<u64> {
        match self {
            Self::Fresh => None,
            Self::JournalRecovery { generation }
            | Self::ApprovalContinuation { generation, .. } => Some(*generation),
        }
    }

    pub fn approval_id(&self) -> Option<ApprovalId> {
        match self {
            Self::ApprovalContinuation { approval_id, .. } => Some(*approval_id),
            Self::Fresh | Self::JournalRecovery { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryHarnessRequest {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub execution: HarnessExecution,
    pub capability: String,
    pub scope_id: ScopeId,
    pub command: String,
}

/// A model-agent proposal effect preserves the complete validated request
/// through governance and effect execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryHarnessProposalRequest {
    pub proposal: crate::model_agent::ModelAgentProposalRequest,
}

impl QueryHarnessProposalRequest {
    pub fn run_id(&self) -> HarnessRunId {
        self.proposal.run_id
    }
}

/// The subject of a validation effect: a task or a single claim, never both
/// and never neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationTarget {
    Task(TaskId),
    Claim(ClaimId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunValidationRequest {
    pub target: ValidationTarget,
    pub validation_report_id: ValidationReportId,
}

impl RunValidationRequest {
    pub fn task_id(&self) -> Option<TaskId> {
        match self.target {
            ValidationTarget::Task(task_id) => Some(task_id),
            ValidationTarget::Claim(_) => None,
        }
    }

    pub fn claim_id(&self) -> Option<ClaimId> {
        match self.target {
            ValidationTarget::Task(_) => None,
            ValidationTarget::Claim(claim_id) => Some(claim_id),
        }
    }
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
    ParseArtifact(ParseArtifactRequest),
    Ocr(OcrEffect),
    IndexFullText(IndexFullTextRequest),
    IndexVector(IndexVectorRequest),
    UpdateGraph(UpdateGraphRequest),
    QueryHarnessProposal(QueryHarnessProposalRequest),
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
