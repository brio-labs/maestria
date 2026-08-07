use std::path::Path;

use axum::{extract::State, response::Json};
use maestria_daemon::api::{ClientOperation, ClientResponse, NotebookListResponse, StatusResponse};
use serde::Serialize;

use super::{
    error::{ProblemCode, StudioError},
    state::StudioState,
};
use crate::agent::AgentProfile;

#[derive(Debug, Serialize)]
pub struct BootstrapResponse {
    pub status: SanitizedStatus,
    pub notebooks: NotebookListResponse,
    pub agents: Vec<AgentProfile>,
}

#[derive(Debug, Serialize)]
pub struct SanitizedStatus {
    pub instance_root: String,
    pub event_count: usize,
    pub task_count: usize,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon bootstrap queries.
pub async fn get(State(state): State<StudioState>) -> Result<Json<BootstrapResponse>, StudioError> {
    let status = state.client.request(ClientOperation::Status).await?;
    let status = match status {
        ClientResponse::Status(status) => sanitize_status(status),
        _ => return Err(StudioError::new(ProblemCode::Internal)),
    };
    let notebooks = state.client.request(ClientOperation::NotebookList).await?;
    let notebooks = match notebooks {
        ClientResponse::NotebookList(notebooks) => notebooks,
        _ => return Err(StudioError::new(ProblemCode::Internal)),
    };
    Ok(Json(BootstrapResponse {
        status,
        notebooks,
        agents: vec![state.agent.profile()],
    }))
}

fn sanitize_status(status: StatusResponse) -> SanitizedStatus {
    let instance_root = Path::new(&status.instance_root)
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| "instance".to_owned(), ToOwned::to_owned);
    SanitizedStatus {
        instance_root,
        event_count: status.event_count,
        task_count: status.task_count,
    }
}
