use axum::{extract::State, response::Json};
use maestria_daemon::api::{ClientOperation, ClientResponse};
use serde::Deserialize;

use super::{
    error::StudioError,
    extract::{ApiJson, ApiPath},
    state::StudioState,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftCreate {
    pub title: String,
    pub markdown: String,
    pub evidence_ids: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftUpdate {
    pub expected_revision: u64,
    pub title: String,
    pub markdown: String,
    pub evidence_ids: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftDelete {
    pub expected_revision: u64,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon draft list request.
pub async fn list(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookDraftList { notebook_id })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels draft creation.
pub async fn create(
    State(state): State<StudioState>,
    ApiPath(notebook_id): ApiPath<u64>,
    ApiJson(input): ApiJson<DraftCreate>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookDraftSave {
                notebook_id,
                draft_id: None,
                expected_revision: None,
                title: input.title,
                markdown: input.markdown,
                evidence_ids: input.evidence_ids,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels the daemon draft lookup.
pub async fn get(
    State(state): State<StudioState>,
    ApiPath((notebook_id, draft_id)): ApiPath<(u64, u64)>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookDraftGet {
                notebook_id,
                draft_id,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels draft updates.
pub async fn update(
    State(state): State<StudioState>,
    ApiPath((notebook_id, draft_id)): ApiPath<(u64, u64)>,
    ApiJson(input): ApiJson<DraftUpdate>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookDraftSave {
                notebook_id,
                draft_id: Some(draft_id),
                expected_revision: Some(input.expected_revision),
                title: input.title,
                markdown: input.markdown,
                evidence_ids: input.evidence_ids,
            })
            .await?,
    ))
}

/// # Cancellation
///
/// Dropping the future cancels draft deletion.
pub async fn delete(
    State(state): State<StudioState>,
    ApiPath((notebook_id, draft_id)): ApiPath<(u64, u64)>,
    ApiJson(input): ApiJson<DraftDelete>,
) -> Result<Json<ClientResponse>, StudioError> {
    Ok(Json(
        state
            .client
            .request(ClientOperation::NotebookDraftDelete {
                notebook_id,
                draft_id,
                expected_revision: input.expected_revision,
            })
            .await?,
    ))
}
