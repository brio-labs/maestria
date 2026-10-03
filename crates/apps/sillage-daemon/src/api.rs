//! Local authenticated daemon client boundary.

/// Responsibility map:
/// - `federation_previews`: scoped cited excerpt construction and bounded search response framing.
/// - `protocol`: legacy client protocol and DTOs.
/// - `protocol_search_api`: versioned external search-only protocol and client.
/// - `server`: authenticated socket listener and legacy request dispatch.
/// - `server_search_api`: version negotiation and bounded search-only requests.
/// - `services`: dispatch and routing façade over responsibility-specific service siblings.
/// - `token`: local instance credential and socket permissions.
mod federation_previews;
mod protocol;
mod protocol_search_api;
pub(crate) mod server;
mod server_search_api;
mod services;
mod token;

pub use protocol::{
    ClientAuthentication, ClientErrorCode, ClientOperation, ClientRequest, ClientResponse,
    CoverageResponse, DaemonClient, DaemonRequestError, EvidenceResponse, EvidenceSourceResponse,
    FederationCredential, FederationEvidenceResponse, FederationSearchResponse,
    FrozenNotebookCitationResponse, IndexCandidatesResponse, IndexRunResponse,
    IndexSelectionResponse, ModelAgentHarnessOutcome, ModelAgentMemoryCandidateSummary,
    ModelAgentProposalPayload, ModelAgentProposalResponse, ModelAgentStatusResponse,
    ModelAgentValidationSummary, NotebookCitationResponse, NotebookContextResponse,
    NotebookDraftDeletedResponse, NotebookDraftListResponse, NotebookDraftResponse,
    NotebookDraftSavedResponse, NotebookDraftSummary, NotebookListResponse, NotebookResponse,
    NotebookSourceCatalogEntry, NotebookSourceCatalogResponse, NotebookSourceSelection,
    NotebookSummary, RealmGrantAccess, RealmGrantCreatedResponse, RealmGrantListResponse,
    RealmGrantResponse, RealmGrantSensitivity, RepositoryIndexCandidatesResponse,
    RepositoryIndexChildrenResponse, RepositoryIndexFile, RepositoryIndexFilesResponse,
    RepositoryIndexProgress, RepositoryIndexProgressResponse, RepositoryIndexRunResponse,
    RepositoryIndexSelectionResponse, RepositoryIndexStatusResponse, RepositoryIndexSummary,
    RetrievalLaneStatus, RetrievalPromotionRecordWire, RetrievalPromotionRecords,
    RetrievalStatusResponse, SearchEvidenceResponse, SearchExcludedSource, SearchIndexingStatus,
    SearchPassagePreviewResponse, SearchRawRankResponse, SearchResponse, SearchRootStatus,
    SearchScoreResponse, SearchScoreScaleResponse, StatusResponse, TaskResponse, TaskSummary,
};
pub use protocol_search_api::{
    SEARCH_API_PROTOCOL, SEARCH_API_VERSION, SEARCH_API_VERSION_2, SearchApiClient,
    SearchApiIndexingStatusResponse, SearchApiOperation, SearchApiResponse,
};
pub use server::ApiServer;

pub(crate) use protocol::ClientReplyOut;
pub(crate) use services::{dispatch, dispatch_search_api};
pub use token::random_hex_credential;
pub(crate) use token::{
    load_or_create_token, remove_stale_socket, set_private_directory_permissions,
    set_private_permissions, socket_path, token_path, validate_token,
};
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
