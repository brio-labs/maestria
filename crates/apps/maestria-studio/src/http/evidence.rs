use axum::{Json, extract::State};
use maestria_daemon::api::{ClientOperation, ClientResponse};

use super::{error::StudioError, extract::ApiPath, state::StudioState};

/// # Cancellation
///
/// Dropping the future cancels the daemon evidence request.
pub async fn evidence_global(
    State(state): State<StudioState>,
    ApiPath(evidence_id): ApiPath<u64>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::Evidence { evidence_id })
            .await?,
    ))
}
