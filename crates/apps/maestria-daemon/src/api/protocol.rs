use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

const MAX_SEARCH_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientOperation {
    Status,
    Search {
        query: String,
        limit: usize,
    },
    Evidence {
        evidence_id: u64,
    },
    Task {
        #[serde(default)]
        task_id: Option<u64>,
    },
    ModelAgentPropose {
        proposal: ModelAgentProposalPayload,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRequest {
    pub token: String,
    pub operation: ClientOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientResponse {
    Status(StatusResponse),
    Search(SearchResponse),
    Evidence(EvidenceResponse),
    Task(TaskResponse),
    ModelAgentProposal(ModelAgentProposalResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub instance_root: String,
    pub event_count: usize,
    pub task_count: usize,
    pub socket_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub query_id: u64,
    pub trace_id: u64,
    pub status: String,
    pub fingerprint: String,
    pub index_generation: u64,
    pub evidence: Vec<SearchEvidenceResponse>,
    pub coverage: CoverageResponse,
    pub conflict_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEvidenceResponse {
    pub evidence_id: u64,
    pub artifact_version: u64,
    pub source: String,
    pub range_start: usize,
    pub range_end: usize,
    pub score_schema_version: u16,
    pub scores: Vec<SearchScoreResponse>,
    pub trust: String,
    pub freshness: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchScoreResponse {
    pub score_kind: String,
    pub raw_score: i64,
    pub raw_rank: SearchRawRankResponse,
    pub scale: SearchScoreScaleResponse,
    pub representation: String,
    pub fingerprint: String,
    pub fingerprint_components: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SearchRawRankResponse {
    Ranked { rank: u32 },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchScoreScaleResponse {
    Binary,
    Unbounded {
        name: String,
        higher_is_better: bool,
    },
    FixedPoint {
        name: String,
        denominator: u32,
        minimum: Option<i64>,
        maximum: Option<i64>,
        higher_is_better: bool,
    },
    RankDerived {
        name: String,
        higher_is_better: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageResponse {
    pub percent_covered: u8,
    pub gaps: Vec<String>,
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceResponse {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    pub artifact_content_hash: Option<String>,
    pub source: EvidenceSourceResponse,
    pub excerpt: String,
    pub observed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceSourceResponse {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
        content_hash: String,
    },
    Pdf {
        snapshot_id: u64,
        page_start: u32,
        page_end: u32,
    },
    PdfRegion {
        snapshot_id: u64,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Web {
        url: String,
        content_hash: String,
        snapshot_id: u64,
    },
    Command {
        harness_run: u64,
        stream: String,
        blob_id: u64,
    },
    Test {
        harness_run: u64,
        status: String,
        log_id: u64,
    },
    Diff {
        harness_run: u64,
        patch_blob_id: u64,
    },
    Validation {
        report_id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResponse {
    pub tasks: Vec<TaskSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: u64,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub evidence_ids: Vec<u64>,
    pub validation_report_id: Option<u64>,
}

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
}

/// Result of a model agent proposal workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAgentProposalResponse {
    pub run_id: u64,
    pub trace_id: Option<u64>,
    pub index_generation: u64,
    pub evidence_count: usize,
    pub harness: Option<ModelAgentHarnessOutcome>,
    pub validation: Option<ModelAgentValidationSummary>,
    pub memory_candidate: Option<ModelAgentMemoryCandidateSummary>,
    pub warnings: Vec<String>,
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

#[derive(Debug, Deserialize)]
pub(crate) struct ClientReply {
    pub(crate) response: Option<ClientResponse>,
    pub(crate) error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ClientReplyOut {
    pub(crate) response: Option<ClientResponse>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
    token: String,
}

impl DaemonClient {
    pub fn from_instance(layout: &InstanceLayout) -> Result<Self> {
        let token_path = super::token_path(layout);
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("read daemon token {}", token_path.display()))?
            .trim()
            .to_string();
        super::validate_token(&token)?;
        Ok(Self {
            socket_path: super::socket_path(layout),
            token,
        })
    }

    pub fn new(socket_path: PathBuf, token_path: PathBuf) -> Result<Self> {
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("read daemon token {}", token_path.display()))?
            .trim()
            .to_string();
        super::validate_token(&token)?;
        Ok(Self { socket_path, token })
    }

    pub async fn request(&self, operation: ClientOperation) -> Result<ClientResponse> {
        if let ClientOperation::Search { limit, .. } = operation
            && !(1..=MAX_SEARCH_LIMIT).contains(&limit)
        {
            return Err(anyhow!(
                "search limit must be between 1 and {MAX_SEARCH_LIMIT}"
            ));
        }
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| format!("connect daemon socket {}", self.socket_path.display()))?;
        let request = ClientRequest {
            token: self.token.clone(),
            operation,
        };
        let mut line = serde_json::to_vec(&request).context("encode daemon request")?;
        line.push(b'\n');
        if line.len() > super::MAX_REQUEST_BYTES {
            return Err(anyhow!("daemon request exceeds size limit"));
        }
        stream
            .write_all(&line)
            .await
            .context("send daemon request")?;
        let mut reader = BufReader::new(stream);
        let mut response_line = Vec::new();
        reader
            .read_until(b'\n', &mut response_line)
            .await
            .context("read daemon response")?;
        if response_line.len() > super::MAX_REQUEST_BYTES {
            return Err(anyhow!("daemon response exceeds size limit"));
        }
        let reply: ClientReply =
            serde_json::from_slice(response_line.trim_ascii()).context("decode daemon response")?;
        match (reply.response, reply.error) {
            (Some(response), None) => Ok(response),
            (None, Some(error)) => Err(anyhow!("daemon request rejected: {error}")),
            _ => Err(anyhow!("daemon returned malformed response envelope")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_agent_proposal_payload_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let payload = ModelAgentProposalPayload {
            run_id: 1,
            task_id: Some(2),
            query: "test query".into(),
            limit: 10,
            capability: "shell".into(),
            command: "echo hello".into(),
            working_directory: "/tmp".into(),
            timeout_secs: 30,
            expected_generation: 4,
            evidence_ids: vec![9],
        };
        let json = serde_json::to_string(&payload)?;
        let deserialized: ModelAgentProposalPayload = serde_json::from_str(&json)?;
        assert_eq!(deserialized.run_id, 1);
        assert_eq!(deserialized.query, "test query");
        Ok(())
    }
}
