use axum::{Json, extract::State};
use maestria_daemon::api::{ClientOperation, ClientResponse};

use super::{error::StudioError, extract::ApiJson, extract::ApiPath, state::StudioState};

/// # Cancellation
///
/// Dropping the future cancels the daemon candidate-scan request.
pub async fn candidates(
    State(state): State<StudioState>,
    ApiPath(root): ApiPath<String>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::IndexCandidates { root })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon selection-load request.
pub async fn selection_get(
    State(state): State<StudioState>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::IndexSelectionGet)
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon selection-save request.
pub async fn selection_save(
    State(state): State<StudioState>,
    ApiJson(profile): ApiJson<maestria_index_selection::IndexSelectionProfile>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::IndexSelectionSave { profile })
            .await?,
    ))
}

/// The wire input for an index run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexRunInput {
    pub root: String,
    pub includes: Vec<String>,
    pub policies: std::collections::BTreeMap<String, maestria_index_selection::IndexPolicy>,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon index-run request.
pub async fn run(
    State(state): State<StudioState>,
    ApiJson(input): ApiJson<IndexRunInput>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::IndexRun {
                root: input.root,
                includes: input.includes,
                policies: input.policies,
            })
            .await?,
    ))
}
