use axum::{extract::State, http::StatusCode, response::Json};
use maestria_daemon::api::{ClientOperation, ClientResponse};
use serde::Deserialize;

use super::{
    error::{ProblemCode, StudioError},
    extract::{ApiJson, ApiPath},
    state::StudioState,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotebookTitle {
    pub title: String,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon notebook request.
pub async fn list(State(state): State<StudioState>) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state.client.request(ClientOperation::NotebookList).await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels notebook creation.
pub async fn create(
    State(state): State<StudioState>,
    ApiJson(input): ApiJson<NotebookTitle>,
) -> Result<(StatusCode, Json<ClientResponse>), StudioError> {
    validate_title(&input.title)?;
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .client
                .request(ClientOperation::NotebookCreate { title: input.title })
                .await?,
        ),
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon notebook lookup.
pub async fn get(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookGet { notebook_id })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels notebook renaming.
pub async fn rename(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
    ApiJson(input): ApiJson<NotebookTitle>,
) -> Result<Json<ClientResponse>, StudioError> {
    validate_title(&input.title)?;
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookRename {
                notebook_id,
                title: input.title,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels notebook deletion.
pub async fn delete(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookDelete { notebook_id })
            .await?,
    ))
}

fn validate_title(title: &str) -> Result<(), StudioError> {
    if title.trim().is_empty() || title.len() > 200 {
        Err(StudioError::new(ProblemCode::InvalidInput))
    } else {
        Ok(())
    }
}
