use axum::{Json, extract::State};
use maestria_daemon::api::{ClientOperation, ClientResponse};

use super::state::StudioState;

/// # Cancellation
///
/// Dropping the future cancels the daemon task list request.
pub async fn list(
    State(state): State<StudioState>,
) -> Result<Json<ClientResponse>, super::error::StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::Task { task_id: None })
            .await?,
    ))
}
