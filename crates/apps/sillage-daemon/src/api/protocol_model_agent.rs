//! Model-agent endpoint payloads (untrusted proposal submission).

use serde::{Deserialize, Serialize};

/// Untrusted proposal payload submitted to the model agent endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentProposalPayload {
    pub run_id: u64,
    pub task_id: Option<u64>,
    pub query: String,
    pub limit: usize,
    pub capability: String,
    pub command: String,
    pub working_directory: String,
    pub timeout_secs: u64,
    pub expected_generation: u64,
    pub evidence_ids: Vec<u64>,
    #[serde(default)]
    pub task_validation: bool,
    #[serde(default)]
    pub memory_candidate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentProposalResponse {
    pub run_id: u64,
    pub correlation_id: u64,
    pub status: String,
    pub approval_id: Option<u64>,
    pub trace_id: Option<u64>,
    pub index_generation: u64,
    pub evidence_count: usize,
    pub harness: Option<ModelAgentHarnessOutcome>,
    pub validation: Option<ModelAgentValidationSummary>,
    pub memory_candidate: Option<ModelAgentMemoryCandidateSummary>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentStatusResponse {
    pub run_id: u64,
    pub correlation_id: Option<u64>,
    pub status: String,
    pub approval_id: Option<u64>,
    pub journal_generation: Option<u64>,
    pub trace_id: Option<u64>,
    pub evidence_count: usize,
    pub harness: Option<ModelAgentHarnessOutcome>,
    pub validation: Option<ModelAgentValidationSummary>,
    pub memory_candidate: Option<ModelAgentMemoryCandidateSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentHarnessOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentValidationSummary {
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentMemoryCandidateSummary {
    pub candidate_id: u64,
    pub confidence_milli: u16,
    pub decision: String,
}
