use crate::ids::{ApprovalId, EvidenceId, HarnessRunId, MemoryCandidateId, TaskId};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelAgentProposalExecution {
    Fresh,
    JournalRecovery {
        journal_generation: u64,
    },
    ApprovalContinuation {
        approval_id: ApprovalId,
        journal_generation: u64,
    },
}

impl ModelAgentProposalExecution {
    pub fn approval_id(&self) -> Option<ApprovalId> {
        match self {
            Self::ApprovalContinuation { approval_id, .. } => Some(*approval_id),
            Self::Fresh | Self::JournalRecovery { .. } => None,
        }
    }

    pub fn journal_generation(&self) -> Option<u64> {
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
    pub expected_generation: u64,
    pub task_validation: bool,
    pub memory_candidate: bool,
    pub execution: ModelAgentProposalExecution,
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
