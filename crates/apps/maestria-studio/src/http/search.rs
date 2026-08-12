use axum::{
    Json,
    extract::{Query, State},
};
use maestria_daemon::api::{ClientOperation, ClientResponse};
use serde::Deserialize;

use super::{error::ProblemCode, error::StudioError, state::StudioState};

const MAX_SEARCH_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchParams {
    pub(crate) query: String,
    pub(crate) limit: Option<usize>,
}

/// # Cancellation
///
/// Dropping the future cancels the daemon search request.
pub async fn search(
    State(state): State<StudioState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<ClientResponse>, StudioError> {
    if params.query.trim().is_empty() {
        return Err(StudioError::new(ProblemCode::InvalidInput));
    }
    let limit = params
        .limit
        .map_or(10, |value| value.clamp(1, MAX_SEARCH_LIMIT));
    Ok(Json(
        state
            .client
            .request(ClientOperation::Search {
                query: params.query,
                limit,
            })
            .await?,
    ))
}
