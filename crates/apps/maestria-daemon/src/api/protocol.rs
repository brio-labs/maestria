use std::collections::BTreeMap;

use maestria_domain::RealmId;
use serde::{Deserialize, Serialize};

#[path = "protocol_client.rs"]
mod protocol_client;
#[path = "protocol_federation.rs"]
mod protocol_federation;

pub use protocol_client::DaemonClient;
pub(crate) use protocol_client::{ClientReplyOut, read_capped_ndjson_line};
pub use protocol_federation::{
    ClientAuthentication, FederationCredential, FederationEvidenceResponse,
    FederationSearchResponse, RealmGrantAccess, RealmGrantCreatedResponse, RealmGrantListResponse,
    RealmGrantResponse, RealmGrantSensitivity,
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
    ModelAgentStatus {
        run_id: u64,
    },
    ModelAgentResolve {
        run_id: u64,
        approval_id: u64,
        approved: bool,
    },
    RealmGrantCreate {
        consumer_realm: RealmId,
        access: RealmGrantAccess,
        max_sensitivity: RealmGrantSensitivity,
        max_results: usize,
        max_evidence_bytes: usize,
    },
    RealmGrantList,
    RealmGrantRevoke {
        token_digest: String,
    },
    InstallFederationBinding {
        provider_realm: RealmId,
        provider_socket_path: String,
        credential: FederationCredential,
    },
    FederationSearch {
        provider_realm: RealmId,
        query: String,
        limit: usize,
    },
    FederationEvidence {
        provider_realm: RealmId,
        evidence_id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRequest {
    pub authentication: ClientAuthentication,
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
    ModelAgentStatus(ModelAgentStatusResponse),
    RealmGrantCreated(RealmGrantCreatedResponse),
    RealmGrantList(RealmGrantListResponse),
    FederationBindingInstalled,
    FederationSearch(FederationSearchResponse),
    FederationEvidence(FederationEvidenceResponse),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

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
            task_validation: true,
            memory_candidate: true,
        };
        let json = serde_json::to_string(&payload)?;
        let deserialized: ModelAgentProposalPayload = serde_json::from_str(&json)?;
        assert_eq!(deserialized.run_id, 1);
        assert_eq!(deserialized.query, "test query");
        Ok(())
    }

    #[test]
    fn federation_credential_is_redacted_and_authentication_is_tagged()
    -> Result<(), Box<dyn std::error::Error>> {
        let credential = FederationCredential::try_from("a".repeat(64))?;
        let request = ClientRequest {
            authentication: ClientAuthentication::FederationGrant {
                consumer_realm: RealmId::try_from("b".repeat(64))?,
                credential: credential.clone(),
            },
            operation: ClientOperation::FederationSearch {
                provider_realm: RealmId::try_from("c".repeat(64))?,
                query: "needle".to_string(),
                limit: 1,
            },
        };

        let encoded = serde_json::to_string(&request)?;

        assert!(encoded.contains(r#""authentication":{"type":"federation_grant""#));
        assert!(!format!("{credential:?}").contains(credential.as_str()));
        let instance_authentication = ClientAuthentication::InstanceToken {
            token: "owner-secret".to_string(),
        };
        assert!(!format!("{instance_authentication:?}").contains("owner-secret"));
        Ok(())
    }

    #[tokio::test]
    async fn capped_ndjson_reader_rejects_unterminated_oversized_message()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut writer, mut reader) = tokio::io::duplex(super::super::MAX_REQUEST_BYTES + 2);
        writer
            .write_all(&vec![b'x'; super::super::MAX_REQUEST_BYTES + 1])
            .await?;
        drop(writer);

        let error = read_capped_ndjson_line(&mut reader)
            .await
            .err()
            .ok_or("oversized message unexpectedly succeeded")?;

        assert!(error.to_string().contains("exceeds maximum length"));
        Ok(())
    }
}
