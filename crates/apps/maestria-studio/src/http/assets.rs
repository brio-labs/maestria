use axum::{
    body::Body,
    extract::OriginalUri,
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    response::Response,
};
use rust_embed::RustEmbed;

use super::error::{ProblemCode, StudioError};

#[derive(RustEmbed)]
#[folder = "../../../web/dist/"]
struct Assets;

/// # Cancellation
///
/// Dropping the future cancels asset lookup and response construction.
pub async fn frontend(uri: OriginalUri) -> Result<Response, StudioError> {
    let requested = uri
        .path()
        .strip_prefix('/')
        .map_or("index.html", |value| value);
    let key = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    let (asset, served_path) = match Assets::get(key) {
        Some(asset) => (asset, key),
        None => {
            let Some(asset) = Assets::get("index.html") else {
                return Err(StudioError::new(ProblemCode::NotFound));
            };
            (asset, "index.html")
        }
    };
    let content_type = content_type(served_path);
    let mut response = Response::new(Body::from(asset.data.into_owned()));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    Ok(response)
}

fn content_type(path: &str) -> &'static str {
    match mime_guess::from_path(path).first_raw() {
        Some("text/html") => "text/html; charset=utf-8",
        Some("text/javascript") | Some("application/javascript") => {
            "text/javascript; charset=utf-8"
        }
        Some("text/css") => "text/css; charset=utf-8",
        Some("application/wasm") => "application/wasm",
        Some(value) => value,
        None => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wasm_content_type_is_stable() {
        assert_eq!(super::content_type("loader.wasm"), "application/wasm");
    }
}
