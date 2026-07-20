//! Local authenticated daemon client boundary.

/// Responsibility map:
/// - `protocol`: module responsibility.
/// - `server`: module responsibility.
/// - `services`: module responsibility.
/// - `token`: module responsibility.
mod protocol;
mod server;
mod services;
mod token;

pub use protocol::{
    ClientOperation, ClientRequest, ClientResponse, CoverageResponse, DaemonClient,
    EvidenceResponse, EvidenceSourceResponse, ModelAgentHarnessOutcome,
    ModelAgentMemoryCandidateSummary, ModelAgentProposalPayload, ModelAgentProposalResponse,
    ModelAgentValidationSummary, SearchEvidenceResponse, SearchResponse, StatusResponse,
    TaskResponse, TaskSummary,
};
pub use server::ApiServer;

pub(crate) use protocol::ClientReplyOut;
pub(crate) use services::dispatch;
pub(crate) use token::{
    load_or_create_token, remove_stale_socket, set_private_directory_permissions,
    set_private_permissions, socket_path, token_path, validate_token,
};
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
