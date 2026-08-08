use maestria_ports::RetentionPolicy;
use url::Url;

use super::{CoreError, CoreResult};

pub(crate) fn string_or_empty(value: &Option<String>) -> String {
    match value {
        Some(value) => value.clone(),
        None => String::new(),
    }
}

pub(crate) fn retention_policy_name(policy: &RetentionPolicy) -> &'static str {
    match policy {
        RetentionPolicy::NoRetention => "no_retention",
        RetentionPolicy::ProviderDefined => "provider_defined",
    }
}

pub(crate) fn parse_retention_policy(value: &str) -> CoreResult<RetentionPolicy> {
    match value {
        "no_retention" => Ok(RetentionPolicy::NoRetention),
        "provider_defined" => Ok(RetentionPolicy::ProviderDefined),
        _ => Err(CoreError::InvalidManifest {
            key: "retention_policy".to_string(),
            reason: format!("invalid value: {value}"),
        }),
    }
}

pub(crate) fn validate_embedding_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidManifest {
        key: "embedding_endpoint".to_string(),
        reason: format!("invalid URL: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/embeddings"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidManifest {
            key: "embedding_endpoint".to_string(),
            reason: "must be an http loopback /v1/embeddings URL".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_ocr_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidManifest {
        key: "ocr_endpoint".to_string(),
        reason: format!("invalid URL: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/chat/completions"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidManifest {
            key: "ocr_endpoint".to_string(),
            reason: "must be an http loopback /v1/chat/completions URL".to_string(),
        });
    }
    Ok(())
}
