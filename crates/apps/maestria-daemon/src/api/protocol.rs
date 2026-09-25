use maestria_domain::RealmId;
use serde::{Deserialize, Serialize};

#[path = "protocol_client.rs"]
mod protocol_client;
#[path = "protocol_federation.rs"]
mod protocol_federation;
#[path = "protocol_index.rs"]
mod protocol_index;
#[path = "protocol_model_agent.rs"]
mod protocol_model_agent;
#[path = "protocol_notebook.rs"]
mod protocol_notebook;
#[path = "protocol_read.rs"]
mod protocol_read;
#[path = "protocol_repository_index.rs"]
mod protocol_repository_index;
#[path = "protocol_retention.rs"]
mod protocol_retention;
pub use protocol_client::{ClientErrorCode, DaemonClient, DaemonRequestError};
pub(crate) use protocol_client::{ClientReplyOut, read_capped_ndjson_line};
pub use protocol_federation::{
    ClientAuthentication, FederationCredential, FederationEvidenceResponse,
    FederationSearchResponse, RealmGrantAccess, RealmGrantCreatedResponse, RealmGrantListResponse,
    RealmGrantResponse, RealmGrantSensitivity,
};
pub use protocol_index::{IndexCandidatesResponse, IndexRunResponse, IndexSelectionResponse};
pub use protocol_model_agent::{
    ModelAgentHarnessOutcome, ModelAgentMemoryCandidateSummary, ModelAgentProposalPayload,
    ModelAgentProposalResponse, ModelAgentStatusResponse, ModelAgentValidationSummary,
};
pub use protocol_notebook::{
    FrozenNotebookCitationResponse, NotebookCitationResponse, NotebookContextResponse,
    NotebookDraftDeletedResponse, NotebookDraftListResponse, NotebookDraftResponse,
    NotebookDraftSavedResponse, NotebookDraftSummary, NotebookListResponse, NotebookResponse,
    NotebookSourceCatalogEntry, NotebookSourceCatalogResponse, NotebookSourceSelection,
    NotebookSummary,
};
pub use protocol_read::{
    CoverageResponse, EvidenceResponse, EvidenceSourceResponse, RetrievalLaneStatus,
    RetrievalPromotionRecordWire, RetrievalPromotionRecords, RetrievalStatusResponse,
    SearchEvidenceResponse, SearchExcludedSource, SearchIndexingStatus,
    SearchPassagePreviewResponse, SearchPathResultResponse, SearchRawRankResponse, SearchResponse,
    SearchRootStatus, SearchRootsStatusResponse, SearchScoreResponse, SearchScoreScaleResponse,
    StatusResponse, TaskResponse, TaskSummary,
};
pub use protocol_repository_index::{
    RepositoryIndexCandidatesResponse, RepositoryIndexChildrenResponse, RepositoryIndexFile,
    RepositoryIndexFilesResponse, RepositoryIndexProgress, RepositoryIndexProgressResponse,
    RepositoryIndexRunResponse, RepositoryIndexSelectionResponse, RepositoryIndexStatusResponse,
    RepositoryIndexSummary,
};
pub use protocol_retention::RetrievalEventsRetiredResponse;
pub(crate) const MAX_SEARCH_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientOperation {
    Status,
    RetrievalStatus,
    SearchRootsStatus,
    SearchRootAdd {
        root: String,
    },
    SearchRootRemove {
        root: String,
    },
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
        #[serde(default)]
        allowed_roots: Vec<String>,
        max_results: usize,
        max_evidence_bytes: usize,
        expires_in_seconds: u64,
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
    NotebookList,
    NotebookCreate {
        title: String,
    },
    NotebookGet {
        notebook_id: u64,
    },
    NotebookRename {
        notebook_id: u64,
        title: String,
    },
    NotebookDelete {
        notebook_id: u64,
    },
    NotebookSourceCatalog {
        query: Option<String>,
        offset: usize,
        limit: usize,
    },
    NotebookSourceAttach {
        notebook_id: u64,
        source_key: String,
    },
    NotebookSourceDetach {
        notebook_id: u64,
        source_key: String,
    },
    NotebookContext {
        notebook_id: u64,
        query: String,
        limit: usize,
        max_context_bytes: usize,
    },
    NotebookEvidence {
        notebook_id: u64,
        evidence_id: u64,
    },
    NotebookDraftList {
        notebook_id: u64,
    },
    NotebookDraftGet {
        notebook_id: u64,
        draft_id: u64,
    },
    NotebookDraftSave {
        notebook_id: u64,
        draft_id: Option<u64>,
        expected_revision: Option<u64>,
        title: String,
        markdown: String,
        evidence_ids: Vec<u64>,
    },
    NotebookDraftDelete {
        notebook_id: u64,
        draft_id: u64,
        expected_revision: u64,
    },
    FederationEvidence {
        provider_realm: RealmId,
        evidence_id: u64,
    },
    IndexCandidates {
        root: String,
    },
    IndexSelectionGet,
    IndexSelectionSave {
        profile: maestria_index_selection::IndexSelectionProfile,
    },
    IndexRun {
        root: String,
        includes: Vec<String>,
        policies: std::collections::BTreeMap<String, maestria_index_selection::IndexPolicy>,
    },
    RepositoryIndexCandidates {
        root: String,
    },
    RepositoryIndexSelectionGet,
    RepositoryIndexSelectionSave {
        profile: maestria_index_selection::IndexSelectionProfile,
    },
    RepositoryIndexRun {
        root: String,
        includes: Vec<String>,
        policies: std::collections::BTreeMap<String, maestria_index_selection::IndexPolicy>,
    },
    RepositoryIndexStatus {
        root: String,
    },
    RepositoryIndexChildren {
        root: String,
        /// Repository-relative directory path to expand.
        path: String,
    },
    RepositoryIndexFiles {
        root: String,
        /// Repository-relative directory path to list.
        path: String,
    },
    RepositoryIndexProgressGet,
    RetireRetrievalEvents {
        before_sequence: u64,
        reason: String,
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
    RetrievalStatus(Box<RetrievalStatusResponse>),
    SearchRootsStatus(SearchRootsStatusResponse),
    Search(SearchResponse),
    Evidence(EvidenceResponse),
    Task(TaskResponse),
    ModelAgentProposal(ModelAgentProposalResponse),
    ModelAgentStatus(ModelAgentStatusResponse),
    RealmGrantCreated(RealmGrantCreatedResponse),
    RealmGrantList(RealmGrantListResponse),
    FederationBindingInstalled,
    FederationSearch(FederationSearchResponse),
    NotebookList(NotebookListResponse),
    Notebook(NotebookResponse),
    NotebookSources(NotebookSourceCatalogResponse),
    NotebookContext(NotebookContextResponse),
    NotebookEvidence(EvidenceResponse),
    NotebookDrafts(NotebookDraftListResponse),
    NotebookDraft(NotebookDraftResponse),
    NotebookDraftSaved(NotebookDraftSavedResponse),
    NotebookDraftDeleted(NotebookDraftDeletedResponse),
    NotebookDeleted,
    FederationEvidence(FederationEvidenceResponse),
    IndexCandidates(IndexCandidatesResponse),
    IndexSelection(IndexSelectionResponse),
    IndexSelectionSaved,
    IndexRun(IndexRunResponse),
    RepositoryIndexCandidates(RepositoryIndexCandidatesResponse),
    RepositoryIndexSelection(RepositoryIndexSelectionResponse),
    RepositoryIndexSelectionSaved,
    RepositoryIndexRun(RepositoryIndexRunResponse),
    RepositoryIndexStatus(RepositoryIndexStatusResponse),
    RepositoryIndexChildren(RepositoryIndexChildrenResponse),
    RepositoryIndexFiles(RepositoryIndexFilesResponse),
    RepositoryIndexProgress(RepositoryIndexProgressResponse),
    RetrievalEventsRetired(RetrievalEventsRetiredResponse),
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
    fn realm_grant_protocol_defaults_missing_root_scope() -> Result<(), Box<dyn std::error::Error>>
    {
        let operation = ClientOperation::RealmGrantCreate {
            consumer_realm: maestria_test_support::realm_id(11)?,
            access: RealmGrantAccess::SearchOnly,
            max_sensitivity: RealmGrantSensitivity::Public,
            allowed_roots: Vec::new(),
            max_results: 1,
            max_evidence_bytes: 1,
            expires_in_seconds: 60,
        };
        let mut legacy_operation = serde_json::to_value(operation)?;
        legacy_operation
            .as_object_mut()
            .ok_or("realm-grant operation was not serialized as an object")?
            .remove("allowed_roots");

        let decoded: ClientOperation = serde_json::from_value(legacy_operation)?;
        let ClientOperation::RealmGrantCreate { allowed_roots, .. } = decoded else {
            return Err("decoded operation was not a realm-grant creation".into());
        };
        assert!(allowed_roots.is_empty());

        let mut legacy_response = serde_json::to_value(RealmGrantResponse {
            token_digest: "a".repeat(64),
            provider_realm: maestria_test_support::realm_id(10)?,
            consumer_realm: maestria_test_support::realm_id(11)?,
            access: RealmGrantAccess::SearchOnly,
            max_sensitivity: RealmGrantSensitivity::Public,
            allowed_roots: None,
            max_results: 1,
            max_evidence_bytes: 1,
            expires_at_unix_seconds: 2,
            state: "active".to_string(),
        })?;
        legacy_response
            .as_object_mut()
            .ok_or("realm-grant response was not serialized as an object")?
            .remove("allowed_roots");
        let decoded_response: RealmGrantResponse = serde_json::from_value(legacy_response)?;
        assert_eq!(decoded_response.allowed_roots, None);
        Ok(())
    }

    #[test]
    fn federation_credential_is_redacted_and_authentication_is_tagged()
    -> Result<(), Box<dyn std::error::Error>> {
        let credential = FederationCredential::try_from("a".repeat(64))?;
        let request = ClientRequest {
            authentication: ClientAuthentication::FederationGrant {
                consumer_realm: maestria_test_support::realm_id(11)?,
                credential: credential.clone(),
            },
            operation: ClientOperation::FederationSearch {
                provider_realm: maestria_test_support::realm_id(12)?,
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
