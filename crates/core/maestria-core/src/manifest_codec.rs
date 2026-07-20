use std::path::PathBuf;

use maestria_ports::RetentionPolicy;
use url::Url;

use crate::error::{CoreError, CoreResult};

pub(super) struct ManifestFields {
    pub(super) schema_version: Option<u32>,
    pub(super) root: Option<PathBuf>,
    pub(super) read_roots: Vec<PathBuf>,
    pub(super) excluded_patterns: Vec<String>,
    embedding_enabled: Option<bool>,
    embedding_endpoint: Option<String>,
    embedding_model: Option<String>,
    embedding_dimensions: Option<usize>,
    embedding_provider: Option<String>,
    embedding_revision: Option<String>,
    embedding_artifact_hash: Option<String>,
    embedding_preprocessing_version: Option<String>,
    embedding_remote_provider: Option<bool>,
    embedding_retention_policy: Option<String>,
    ocr_enabled: Option<bool>,
    ocr_endpoint: Option<String>,
    ocr_model: Option<String>,
    ocr_provider: Option<String>,
    ocr_revision: Option<String>,
    ocr_artifact_hash: Option<String>,
    ocr_preprocessing_version: Option<String>,
}

pub(super) fn parse_ocr_config(fields: &ManifestFields) -> CoreResult<Option<super::OcrConfig>> {
    match (&fields.ocr_enabled, &fields.ocr_endpoint, &fields.ocr_model) {
        (None, None, None) => Ok(None),
        (Some(enabled), Some(endpoint), Some(model)) => {
            validate_ocr_endpoint(endpoint)?;
            let provider = if *enabled {
                fields
                    .ocr_provider
                    .clone()
                    .ok_or_else(ocr_fingerprint_error)?
            } else {
                string_or_empty(&fields.ocr_provider)
            };
            let revision = if *enabled {
                fields
                    .ocr_revision
                    .clone()
                    .ok_or_else(ocr_fingerprint_error)?
            } else {
                string_or_empty(&fields.ocr_revision)
            };
            let artifact_hash = if *enabled {
                fields
                    .ocr_artifact_hash
                    .clone()
                    .ok_or_else(ocr_fingerprint_error)?
            } else {
                string_or_empty(&fields.ocr_artifact_hash)
            };
            let preprocessing_version = if *enabled {
                fields
                    .ocr_preprocessing_version
                    .clone()
                    .ok_or_else(ocr_fingerprint_error)?
            } else {
                string_or_empty(&fields.ocr_preprocessing_version)
            };
            if *enabled {
                maestria_domain::ContentHash::new(artifact_hash.clone()).map_err(|error| {
                    CoreError::InvalidInput {
                        message: format!("invalid OCR artifact hash: {error}"),
                    }
                })?;
            }
            Ok(Some(super::OcrConfig {
                enabled: *enabled,
                endpoint: endpoint.clone(),
                model: model.clone(),
                provider,
                revision,
                artifact_hash,
                preprocessing_version,
            }))
        }
        _ => Err(CoreError::InvalidInput {
            message: "OCR configuration must define enabled, endpoint, and model".to_string(),
        }),
    }
}

fn ocr_fingerprint_error() -> CoreError {
    CoreError::InvalidInput {
        message:
            "configured OCR requires provider, revision, artifact hash, and preprocessing version"
                .to_string(),
    }
}

pub(super) fn parse_embedding_config(
    fields: &ManifestFields,
) -> CoreResult<Option<super::EmbeddingConfig>> {
    match (
        &fields.embedding_enabled,
        &fields.embedding_endpoint,
        &fields.embedding_model,
        &fields.embedding_dimensions,
    ) {
        (None, None, None, None) => Ok(None),
        (Some(enabled), Some(endpoint), Some(model), Some(dimensions)) => {
            validate_embedding_endpoint(endpoint)?;
            if *enabled && *dimensions == 0 {
                return Err(CoreError::InvalidInput {
                    message: "embedding_dimensions must be positive when enabled".to_string(),
                });
            }
            let provider = if *enabled {
                fields
                    .embedding_provider
                    .clone()
                    .ok_or_else(embedding_fingerprint_error)?
            } else {
                string_or_empty(&fields.embedding_provider)
            };
            let revision = if *enabled {
                fields
                    .embedding_revision
                    .clone()
                    .ok_or_else(embedding_fingerprint_error)?
            } else {
                string_or_empty(&fields.embedding_revision)
            };
            let artifact_hash = if *enabled {
                fields
                    .embedding_artifact_hash
                    .clone()
                    .ok_or_else(embedding_fingerprint_error)?
            } else {
                string_or_empty(&fields.embedding_artifact_hash)
            };
            let preprocessing_version = if *enabled {
                fields
                    .embedding_preprocessing_version
                    .clone()
                    .ok_or_else(embedding_fingerprint_error)?
            } else {
                string_or_empty(&fields.embedding_preprocessing_version)
            };
            if *enabled {
                maestria_domain::ContentHash::new(artifact_hash.clone()).map_err(|error| {
                    CoreError::InvalidInput {
                        message: format!("invalid embedding artifact hash: {error}"),
                    }
                })?;
            }
            Ok(Some(super::EmbeddingConfig {
                enabled: *enabled,
                endpoint: endpoint.clone(),
                model: model.clone(),
                dimensions: *dimensions,
                provider,
                revision,
                artifact_hash,
                preprocessing_version,
                remote_provider: fields.embedding_remote_provider.is_some_and(|value| value),
                retention_policy: parse_retention_policy(
                    fields
                        .embedding_retention_policy
                        .as_deref()
                        .map_or("no_retention", |value| value),
                )?,
            }))
        }
        _ => Err(CoreError::InvalidInput {
            message: "embedding configuration must define enabled, endpoint, model, and dimensions"
                .to_string(),
        }),
    }
}

fn embedding_fingerprint_error() -> CoreError {
    CoreError::InvalidInput {
        message: "enabled embedding configuration requires provider, revision, artifact hash, and preprocessing version"
            .to_string(),
    }
}

fn string_or_empty(value: &Option<String>) -> String {
    match value {
        Some(value) => value.clone(),
        None => String::new(),
    }
}

pub(super) fn parse_manifest_fields(contents: &str) -> CoreResult<ManifestFields> {
    let mut fields = empty_manifest_fields();
    for line in contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| CoreError::InvalidInput {
                message: format!("invalid instance manifest line: {line}"),
            })?;
        if value.is_empty() {
            return Err(CoreError::InvalidInput {
                message: format!("instance manifest value is empty for {key}"),
            });
        }
        parse_manifest_field(&mut fields, key, value)?;
    }
    Ok(fields)
}

fn empty_manifest_fields() -> ManifestFields {
    ManifestFields {
        schema_version: None,
        root: None,
        read_roots: Vec::new(),
        excluded_patterns: Vec::new(),
        embedding_enabled: None,
        embedding_endpoint: None,
        embedding_model: None,
        embedding_dimensions: None,
        embedding_provider: None,
        embedding_revision: None,
        embedding_artifact_hash: None,
        embedding_preprocessing_version: None,
        embedding_remote_provider: None,
        embedding_retention_policy: None,
        ocr_enabled: None,
        ocr_endpoint: None,
        ocr_model: None,
        ocr_provider: None,
        ocr_revision: None,
        ocr_artifact_hash: None,
        ocr_preprocessing_version: None,
    }
}

fn parse_manifest_field(fields: &mut ManifestFields, key: &str, value: &str) -> CoreResult<()> {
    match key {
        "schema_version" => fields.schema_version = Some(parse_value(value, key)?),
        "root" => fields.root = Some(PathBuf::from(value)),
        "read_root" => fields.read_roots.push(PathBuf::from(value)),
        "excluded_pattern" => fields.excluded_patterns.push(value.to_string()),
        "embedding_enabled" => fields.embedding_enabled = Some(parse_value(value, key)?),
        "embedding_endpoint" => fields.embedding_endpoint = Some(value.to_string()),
        "embedding_provider" => fields.embedding_provider = Some(value.to_string()),
        "embedding_revision" => fields.embedding_revision = Some(value.to_string()),
        "embedding_artifact_hash" => fields.embedding_artifact_hash = Some(value.to_string()),
        "embedding_preprocessing_version" => {
            fields.embedding_preprocessing_version = Some(value.to_string());
        }
        "embedding_model" => fields.embedding_model = Some(value.to_string()),
        "embedding_dimensions" => fields.embedding_dimensions = Some(parse_value(value, key)?),
        "embedding_remote_provider" => {
            fields.embedding_remote_provider = Some(parse_value(value, key)?);
        }
        "embedding_retention_policy" => {
            fields.embedding_retention_policy = Some(value.to_string());
        }
        "ocr_enabled" => fields.ocr_enabled = Some(parse_value(value, key)?),
        "ocr_endpoint" => fields.ocr_endpoint = Some(value.to_string()),
        "ocr_model" => fields.ocr_model = Some(value.to_string()),
        "ocr_provider" => fields.ocr_provider = Some(value.to_string()),
        "ocr_revision" => fields.ocr_revision = Some(value.to_string()),
        "ocr_artifact_hash" => fields.ocr_artifact_hash = Some(value.to_string()),
        "ocr_preprocessing_version" => {
            fields.ocr_preprocessing_version = Some(value.to_string());
        }
        other => {
            return Err(CoreError::InvalidInput {
                message: format!("unknown instance manifest key: {other}"),
            });
        }
    }
    Ok(())
}

fn parse_value<T>(value: &str, key: &str) -> CoreResult<T>
where
    T: std::str::FromStr,
{
    value.parse::<T>().map_err(|_| CoreError::InvalidInput {
        message: format!("invalid {key} value: {value}"),
    })
}

pub(super) fn retention_policy_name(policy: &RetentionPolicy) -> &'static str {
    match policy {
        RetentionPolicy::NoRetention => "no_retention",
        RetentionPolicy::ProviderDefined => "provider_defined",
    }
}

fn parse_retention_policy(value: &str) -> CoreResult<RetentionPolicy> {
    match value {
        "no_retention" => Ok(RetentionPolicy::NoRetention),
        "provider_defined" => Ok(RetentionPolicy::ProviderDefined),
        _ => Err(CoreError::InvalidInput {
            message: format!("invalid embedding_retention_policy value: {value}"),
        }),
    }
}

fn validate_embedding_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidInput {
        message: format!("invalid embedding endpoint: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/embeddings"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidInput {
            message: "embedding endpoint must be an http loopback /v1/embeddings URL".to_string(),
        });
    }
    Ok(())
}
fn validate_ocr_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidInput {
        message: format!("invalid OCR endpoint: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/chat/completions"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidInput {
            message: "OCR endpoint must be an http loopback /v1/chat/completions URL".to_string(),
        });
    }
    Ok(())
}
