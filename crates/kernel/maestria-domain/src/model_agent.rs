use crate::ids::{
    ApprovalId, CorrelationId, EvidenceId, HarnessRunId, IndexGenerationId, JournalGeneration,
    MemoryCandidateId, SearchTraceId, TaskId,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelAgentProposalExecution {
    Fresh,
    JournalRecovery {
        journal_generation: JournalGeneration,
    },
    ApprovalContinuation {
        approval_id: ApprovalId,
        journal_generation: JournalGeneration,
    },
}

impl ModelAgentProposalExecution {
    pub fn approval_id(&self) -> Option<ApprovalId> {
        match self {
            Self::ApprovalContinuation { approval_id, .. } => Some(*approval_id),
            Self::Fresh | Self::JournalRecovery { .. } => None,
        }
    }

    pub fn journal_generation(&self) -> Option<JournalGeneration> {
        match self {
            Self::Fresh => None,
            Self::JournalRecovery { journal_generation }
            | Self::ApprovalContinuation {
                journal_generation, ..
            } => Some(*journal_generation),
        }
    }
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
    pub expected_generation: IndexGenerationId,
    pub task_validation: bool,
    pub memory_candidate: bool,
    pub execution: ModelAgentProposalExecution,
    pub correlation_id: CorrelationId,
}

impl ModelAgentProposalRequest {
    pub fn into_harness_request(self) -> crate::effects::QueryHarnessProposalRequest {
        crate::effects::QueryHarnessProposalRequest { proposal: self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAgentSearchResult {
    pub trace_id: SearchTraceId,
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

/// Terminal outcome of a model-agent proposal run.
///
/// `Succeeded` carries every completed stage (search, harness, validation,
/// memory candidate) as independently optional data; `Failed` carries the
/// terminal error and no stage results. The failure path is exclusive with
/// stage results, so the correlated `status` + `error: Option<String>` pair is
/// modeled as two variants instead of a flag plus coordinated fields.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAgentProposalResult {
    Succeeded {
        run_id: HarnessRunId,
        correlation_id: CorrelationId,
        search: Option<ModelAgentSearchResult>,
        harness: Option<ModelAgentHarnessResult>,
        validation: Option<ModelAgentValidationResult>,
        memory_candidate: Option<ModelAgentMemoryResult>,
    },
    Failed {
        run_id: HarnessRunId,
        correlation_id: CorrelationId,
        error: String,
    },
}

impl ModelAgentProposalResult {
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        match self {
            Self::Succeeded { run_id, .. } | Self::Failed { run_id, .. } => *run_id,
        }
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        match self {
            Self::Succeeded { correlation_id, .. } | Self::Failed { correlation_id, .. } => {
                *correlation_id
            }
        }
    }

    #[must_use]
    pub const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}
