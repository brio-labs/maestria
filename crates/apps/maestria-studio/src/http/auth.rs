use axum::{
    body::Body,
    http::{Method, Request, header},
    middleware::Next,
    response::Response,
};

use super::{error::StudioError, state::StudioState};

pub(crate) async fn security(
    state: axum::extract::State<StudioState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StudioError> {
    check_origin(&state, &request)?;
    check_bearer(&state, &request)?;
    Ok(next.run(request).await)
}

fn check_origin(state: &StudioState, request: &Request<Body>) -> Result<(), StudioError> {
    if matches!(*request.method(), Method::GET | Method::HEAD) {
        return Ok(());
    }
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if origin == Some(state.origin.as_ref()) {
        Ok(())
    } else {
        Err(StudioError::new(super::error::ProblemCode::OriginDenied))
    }
}

fn check_bearer(state: &StudioState, request: &Request<Body>) -> Result<(), StudioError> {
    let is_public_static = matches!(*request.method(), Method::GET | Method::HEAD)
        && !request.uri().path().starts_with("/api/");
    if is_public_static {
        return Ok(());
    }
    let expected = format!("Bearer {}", state.bearer);
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if supplied == Some(expected.as_str()) {
        Ok(())
    } else {
        Err(StudioError::new(super::error::ProblemCode::Unauthorized))
    }
}
