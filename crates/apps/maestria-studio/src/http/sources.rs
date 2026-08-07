use axum::{extract::State, response::Json};
use maestria_daemon::api::{ClientOperation, ClientResponse};

use super::{error::StudioError, extract::ApiPath, state::StudioState};

/// # Cancellation
///
/// Dropping the future cancels the daemon source catalog request.
pub async fn catalog(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
) -> Result<Json<ClientResponse>, StudioError> {
    let _ = notebook_id;
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookSourceCatalog {
                query: None,
                offset: 0,
                limit: 100,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels source attachment.
pub async fn attach(
    State(state): State<StudioState>,
    ApiPath((notebook_id, source_key)): ApiPath<(u64, String)>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookSourceAttach {
                notebook_id,
                source_key,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels source detachment.
pub async fn detach(
    State(state): State<StudioState>,
    ApiPath((notebook_id, source_key)): ApiPath<(u64, String)>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookSourceDetach {
                notebook_id,
                source_key,
            })
            .await?,
    ))
}
