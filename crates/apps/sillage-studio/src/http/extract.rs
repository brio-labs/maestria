use std::error::Error;

use axum::{
    body::Body,
    extract::{FromRequest, FromRequestParts, Json, Path},
    http::{Request, request::Parts},
};
use serde::de::DeserializeOwned;
use tower_http::timeout::TimeoutError;

use super::error::{ProblemCode, StudioError};

pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S, Body> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = StudioError;

    async fn from_request(request: Request<Body>, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                let code = if rejection.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                    ProblemCode::RequestTooLarge
                } else if rejection.status() == axum::http::StatusCode::REQUEST_TIMEOUT
                    || has_timeout_source(&rejection)
                {
                    ProblemCode::RequestTimeout
                } else {
                    ProblemCode::InvalidInput
                };
                Err(StudioError::new(code))
            }
        }
    }
}
fn has_timeout_source(error: &dyn Error) -> bool {
    let mut source = error.source();
    while let Some(current) = source {
        if current.is::<TimeoutError>() {
            return true;
        }
        source = current.source();
    }
    false
}

pub struct ApiPath<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = StudioError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| StudioError::new(ProblemCode::InvalidInput))
    }
}
