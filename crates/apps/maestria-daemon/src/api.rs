//! Local authenticated daemon client boundary.

/// Responsibility map:
/// - `protocol`: module responsibility.
/// - `server`: module responsibility.
/// - `services`: dispatch and routing façade over responsibility-specific service siblings.
/// - `token`: module responsibility.
mod protocol;
pub(crate) mod server;
mod services;
mod token;

pub use protocol::{
    ClientAuthentication, ClientErrorCode, ClientOperation, ClientRequest, ClientResponse,
    CoverageResponse, DaemonClient, DaemonRequestError, EvidenceResponse, EvidenceSourceResponse,
    FederationCredential, FederationEvidenceResponse, FederationSearchResponse,
    FrozenNotebookCitationResponse, ModelAgentHarnessOutcome, ModelAgentMemoryCandidateSummary,
    ModelAgentProposalPayload, ModelAgentProposalResponse, ModelAgentStatusResponse,
    ModelAgentValidationSummary, NotebookCitationResponse, NotebookContextResponse,
    NotebookDraftDeletedResponse, NotebookDraftListResponse, NotebookDraftResponse,
    NotebookDraftSavedResponse, NotebookDraftSummary, NotebookListResponse, NotebookResponse,
    NotebookSourceCatalogEntry, NotebookSourceCatalogResponse, NotebookSourceSelection,
    NotebookSummary, RealmGrantAccess, RealmGrantCreatedResponse, RealmGrantListResponse,
    RealmGrantResponse, RealmGrantSensitivity, RetrievalLaneStatus, RetrievalPromotionRecordWire,
    RetrievalPromotionRecords, RetrievalStatusResponse, SearchEvidenceResponse,
    SearchRawRankResponse, SearchResponse, SearchScoreResponse, SearchScoreScaleResponse,
    StatusResponse, TaskResponse, TaskSummary,
};
pub use server::ApiServer;

pub(crate) use protocol::ClientReplyOut;
pub(crate) use services::dispatch;
pub(crate) use token::{
    load_or_create_token, remove_stale_socket, set_private_directory_permissions,
    set_private_permissions, socket_path, token_path, validate_token,
};
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
