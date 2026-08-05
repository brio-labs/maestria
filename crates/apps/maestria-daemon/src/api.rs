//! Local authenticated daemon client boundary.

/// Responsibility map:
/// - `protocol`: module responsibility.
/// - `server`: module responsibility.
/// - `services`: dispatch and routing façade over responsibility-specific service siblings.
/// - `token`: module responsibility.
mod protocol;
mod server;
mod services;
mod token;

pub use protocol::{
    ClientAuthentication, ClientOperation, ClientRequest, ClientResponse, CoverageResponse,
    DaemonClient, EvidenceResponse, EvidenceSourceResponse, FederationCredential,
    FederationEvidenceResponse, FederationSearchResponse, ModelAgentHarnessOutcome,
    ModelAgentMemoryCandidateSummary, ModelAgentProposalPayload, ModelAgentProposalResponse,
    ModelAgentStatusResponse, ModelAgentValidationSummary, RealmGrantAccess,
    RealmGrantCreatedResponse, RealmGrantListResponse, RealmGrantResponse, RealmGrantSensitivity,
    SearchEvidenceResponse, SearchRawRankResponse, SearchResponse, SearchScoreResponse,
    SearchScoreScaleResponse, StatusResponse, TaskResponse, TaskSummary,
};
pub use server::ApiServer;

pub(crate) use protocol::ClientReplyOut;
pub(crate) use services::dispatch;
pub(crate) use token::{
    load_or_create_token, remove_stale_socket, set_private_directory_permissions,
    set_private_permissions, socket_path, token_path, validate_token,
};
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
