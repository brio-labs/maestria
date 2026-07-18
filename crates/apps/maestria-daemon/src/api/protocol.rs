use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

const MAX_SEARCH_LIMIT: usize = 100;

/// A request supported by the local daemon client boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientOperation {
    /// Return instance and projection counts.
    Status,
    /// Execute a bounded, read-only knowledge search.
    Search { query: String, limit: usize },
    /// Open one provenance-verified evidence record.
    Evidence { evidence_id: u64 },
    /// Return one task or all tasks when `task_id` is omitted.
    Task {
        #[serde(default)]
        task_id: Option<u64>,
    },
}

/// An authenticated request sent to the daemon socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRequest {
    /// Per-instance credential read from the daemon token file.
    pub token: String,
    /// Read-only operation to execute.
    pub operation: ClientOperation,
}

/// Typed reply returned by the daemon socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientResponse {
    /// Instance status and counts.
    Status(StatusResponse),
    /// Search plan identity and evidence candidates.
    Search(SearchResponse),
    /// Opened source-backed evidence.
    Evidence(EvidenceResponse),
    /// Task state projection.
    Task(TaskResponse),
}

/// Status data exposed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    /// Canonical instance root.
    pub instance_root: String,
    /// Number of replayed domain events.
    pub event_count: usize,
    /// Number of authoritative tasks.
    pub task_count: usize,
    /// Socket path clients should connect to.
    pub socket_path: String,
}

/// Search data exposed at the API boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Normalized query used by the search planner.
    pub query: String,
    /// Stable query identifier.
    pub query_id: u64,
    /// Stable trace identifier.
    pub trace_id: u64,
    /// Search status name.
    pub status: String,
    /// Retrieval model fingerprint.
    pub fingerprint: String,
    /// Index generation used by the plan.
    pub index_generation: u64,
    /// Evidence candidates returned by retrieval.
    pub evidence: Vec<SearchEvidenceResponse>,
    /// Coverage summary for the returned evidence.
    pub coverage: CoverageResponse,
    /// Number of detected conflict sets.
    pub conflict_count: usize,
}

/// One source-grounded search candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEvidenceResponse {
    /// Evidence identifier clients can pass to `Evidence`.
    pub evidence_id: u64,
    /// Artifact version used by retrieval.
    pub artifact_version: u64,
    /// Human-readable source span.
    pub source: String,
    /// Source character range.
    pub range_start: usize,
    /// Exclusive source character end.
    pub range_end: usize,
    /// Lexical score, if available.
    pub lexical_score: u32,
    /// Dense score, if available.
    pub semantic_score: u32,
    /// Trust label assigned by retrieval.
    pub trust: String,
    /// Freshness label assigned by retrieval.
    pub freshness: String,
}

/// Search evidence coverage at the API boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageResponse {
    /// Percentage of required coverage satisfied.
    pub percent_covered: u8,
    /// Missing coverage requirements.
    pub gaps: Vec<String>,
    /// Distinct source count.
    pub distinct_sources: usize,
    /// Distinct document count.
    pub distinct_documents: usize,
    /// Distinct section count.
    pub distinct_sections: usize,
}

/// Opened evidence and its immutable artifact context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceResponse {
    /// Evidence identifier.
    pub evidence_id: u64,
    /// Artifact identifier owning the evidence.
    pub artifact_id: u64,
    /// Artifact title.
    pub artifact_title: String,
    /// Artifact content hash, when available.
    pub artifact_content_hash: Option<String>,
    /// Evidence kind and provenance fields.
    pub source: EvidenceSourceResponse,
    /// Source excerpt verified by the core service.
    pub excerpt: String,
    /// Logical observation tick.
    pub observed_at: u64,
}

/// Typed evidence provenance at the client boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceSourceResponse {
    /// File line span.
    File {
        path: String,
        start_line: u32,
        end_line: u32,
        content_hash: String,
    },
    /// PDF page span.
    Pdf { page_start: u32, page_end: u32 },
    /// Immutable web snapshot.
    Web {
        url: String,
        content_hash: String,
        snapshot_id: u64,
    },
    /// Harness command output.
    Command {
        harness_run: u64,
        stream: String,
        blob_id: u64,
    },
    /// Harness test result.
    Test {
        harness_run: u64,
        status: String,
        log_id: u64,
    },
    /// Harness diff.
    Diff {
        harness_run: u64,
        patch_blob_id: u64,
    },
    /// Validation report.
    Validation { report_id: u64 },
}

/// Task projection at the client boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResponse {
    /// Returned tasks in ascending identifier order.
    pub tasks: Vec<TaskSummary>,
}

/// One task summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    /// Task identifier.
    pub task_id: u64,
    /// Task title.
    pub title: String,
    /// Current domain status.
    pub status: String,
    /// Task priority.
    pub priority: String,
    /// Evidence linked to the task.
    pub evidence_ids: Vec<u64>,
    /// Validation report linked to the task, if any.
    pub validation_report_id: Option<u64>,
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

/// A supported local client for a running daemon.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
    token: String,
}

impl DaemonClient {
    /// Creates a client from an instance system directory.
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

    /// Creates a client from explicit socket and token paths.
    pub fn new(socket_path: PathBuf, token_path: PathBuf) -> Result<Self> {
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("read daemon token {}", token_path.display()))?
            .trim()
            .to_string();
        super::validate_token(&token)?;
        Ok(Self { socket_path, token })
    }

    /// Sends one bounded request and waits for its typed reply.
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
    fn operation_wire_format_is_tagged_and_typed() -> Result<(), Box<dyn std::error::Error>> {
        let request = ClientRequest {
            token: "a".repeat(64),
            operation: ClientOperation::Search {
                query: "scope".into(),
                limit: 5,
            },
        };
        let encoded = serde_json::to_string(&request)?;
        let decoded: ClientRequest = serde_json::from_str(&encoded)?;
        assert!(matches!(
            decoded.operation,
            ClientOperation::Search { limit: 5, .. }
        ));
        Ok(())
    }
}
