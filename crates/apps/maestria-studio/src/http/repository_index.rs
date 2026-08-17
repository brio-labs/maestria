use axum::{Json, extract::State};
use maestria_daemon::api::{ClientOperation, ClientResponse};

use super::{error::StudioError, extract::ApiJson, extract::ApiPath, state::StudioState};

/// # Cancellation
///
/// Dropping the future cancels the daemon repository candidate-scan request.
pub async fn candidates(
    State(state): State<StudioState>,
    ApiPath(root): ApiPath<String>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexCandidates { root })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon repository selection-load request.
pub async fn selection_get(
    State(state): State<StudioState>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexSelectionGet)
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon repository selection-save request.
pub async fn selection_save(
    State(state): State<StudioState>,
    ApiJson(profile): ApiJson<maestria_index_selection::IndexSelectionProfile>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexSelectionSave { profile })
            .await?,
    ))
}

/// The wire input for a repository code index run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexRunInput {
    pub root: String,
    pub includes: Vec<String>,
    pub policies: std::collections::BTreeMap<String, maestria_index_selection::IndexPolicy>,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon repository index-run request.
pub async fn run(
    State(state): State<StudioState>,
    ApiJson(input): ApiJson<RepositoryIndexRunInput>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexRun {
                root: input.root,
                includes: input.includes,
                policies: input.policies,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon repository index-status request.
pub async fn status(
    State(state): State<StudioState>,
    ApiPath(root): ApiPath<String>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexStatus { root })
            .await?,
    ))
}

/// The wire input for a repository directory expansion or file listing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndexBrowseInput {
    pub root: String,
    /// Repository-relative directory path (empty = the root).
    pub path: String,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon directory-expansion request.
pub async fn children(
    State(state): State<StudioState>,
    ApiJson(input): ApiJson<RepositoryIndexBrowseInput>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexChildren {
                root: input.root,
                path: input.path,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon file-listing request.
pub async fn files(
    State(state): State<StudioState>,
    ApiJson(input): ApiJson<RepositoryIndexBrowseInput>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexFiles {
                root: input.root,
                path: input.path,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon progress request.
pub async fn progress(
    State(state): State<StudioState>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::RepositoryIndexProgressGet)
            .await?,
    ))
}
