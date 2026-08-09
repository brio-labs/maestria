use axum::{Json, extract::State};
use maestria_daemon::api::{ClientOperation, ClientResponse};

use super::state::StudioState;

/// # Cancellation
///
/// Dropping the future cancels the daemon retrieval status request.
pub async fn status(
    State(state): State<StudioState>,
) -> Result<Json<ClientResponse>, super::error::StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RetrievalStatus)
            .await?,
    ))
}
