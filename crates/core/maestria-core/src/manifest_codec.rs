#[path = "manifest_codec_common.rs"]
pub(super) mod common;
#[path = "manifest_codec_visual.rs"]
mod visual;
pub(super) use common::{
    parse_retention_policy, retention_policy_name, string_or_empty, validate_embedding_endpoint,
    validate_ocr_endpoint,
};
use std::path::PathBuf;
pub(super) use visual::parse_visual_config;

use crate::error::{CoreError, CoreResult};

pub(super) struct ManifestFields {
    pub(super) schema_version: Option<u32>,
    pub(super) realm_id: Option<String>,
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
    embedding_query_template: Option<String>,
    embedding_document_template: Option<String>,
    ocr_enabled: Option<bool>,
    ocr_endpoint: Option<String>,
    ocr_model: Option<String>,
    ocr_provider: Option<String>,
    ocr_revision: Option<String>,
    ocr_artifact_hash: Option<String>,
    ocr_preprocessing_version: Option<String>,
    visual_enabled: Option<bool>,
    visual_endpoint: Option<String>,
    visual_model: Option<String>,
    visual_dimensions: Option<usize>,
    visual_provider: Option<String>,
    visual_revision: Option<String>,
    visual_artifact_hash: Option<String>,
    visual_preprocessing_version: Option<String>,
    visual_remote_provider: Option<bool>,
    visual_retention_policy: Option<String>,
    pub(crate) sparse_enabled: Option<bool>,
    pub(crate) sparse_endpoint: Option<String>,
    pub(crate) sparse_provider: Option<String>,
    pub(crate) sparse_revision: Option<String>,
    pub(crate) sparse_artifact_hash: Option<String>,
    pub(crate) sparse_preprocessing_version: Option<String>,
    pub(crate) sparse_model: Option<String>,
    pub(crate) sparse_vocabulary_size: Option<u32>,
    pub(crate) sparse_term_cap: Option<u32>,
    pub(crate) sparse_remote_provider: Option<bool>,
    pub(crate) sparse_retention_policy: Option<String>,
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
                    CoreError::InvalidManifest {
                        key: "ocr_artifact_hash".to_string(),
                        reason: format!("invalid content hash: {error}"),
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
        _ => Err(CoreError::InvalidManifest {
            key: "ocr_config".to_string(),
            reason: "must define enabled, endpoint, and model".to_string(),
        }),
    }
}

fn ocr_fingerprint_error() -> CoreError {
    CoreError::InvalidManifest {
        key: "ocr_config".to_string(),
        reason: "enabled configuration requires provider, revision, artifact hash, and preprocessing version".to_string(),
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
                return Err(CoreError::InvalidManifest {
                    key: "embedding_dimensions".to_string(),
                    reason: "must be positive when enabled".to_string(),
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
                    CoreError::InvalidManifest {
                        key: "embedding_artifact_hash".to_string(),
                        reason: format!("invalid content hash: {error}"),
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
                query_template: match &fields.embedding_query_template {
                    Some(template) => template.clone(),
                    None => "query: {{text}}".to_string(),
                },
                document_template: match &fields.embedding_document_template {
                    Some(template) => template.clone(),
                    None => "document: {{text}}".to_string(),
                },
            }))
        }
        _ => Err(CoreError::InvalidManifest {
            key: "embedding_config".to_string(),
            reason: "must define enabled, endpoint, model, and dimensions".to_string(),
        }),
    }
}

fn embedding_fingerprint_error() -> CoreError {
    CoreError::InvalidManifest {
        key: "embedding_config".to_string(),
        reason: "enabled configuration requires provider, revision, artifact hash, and preprocessing version".to_string(),
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
            .ok_or_else(|| CoreError::InvalidManifest {
                key: "line".to_string(),
                reason: format!("invalid format: {line}"),
            })?;
        if value.is_empty()
            && key != "embedding_query_template"
            && key != "embedding_document_template"
        {
            return Err(CoreError::InvalidManifest {
                key: key.to_string(),
                reason: "value is empty".to_string(),
            });
        }
        parse_manifest_field(&mut fields, key, value)?;
    }
    Ok(fields)
}

fn empty_manifest_fields() -> ManifestFields {
    ManifestFields {
        schema_version: None,
        realm_id: None,
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
        embedding_query_template: None,
        embedding_document_template: None,
        ocr_enabled: None,
        ocr_endpoint: None,
        ocr_model: None,
        ocr_provider: None,
        ocr_revision: None,
        ocr_artifact_hash: None,
        ocr_preprocessing_version: None,
        visual_enabled: None,
        visual_endpoint: None,
        visual_model: None,
        visual_dimensions: None,
        visual_provider: None,
        visual_revision: None,
        visual_artifact_hash: None,
        visual_preprocessing_version: None,
        visual_remote_provider: None,
        visual_retention_policy: None,
        sparse_enabled: None,
        sparse_endpoint: None,
        sparse_provider: None,
        sparse_revision: None,
        sparse_artifact_hash: None,
        sparse_preprocessing_version: None,
        sparse_model: None,
        sparse_vocabulary_size: None,
        sparse_term_cap: None,
        sparse_remote_provider: None,
        sparse_retention_policy: None,
    }
}

fn parse_manifest_field(fields: &mut ManifestFields, key: &str, value: &str) -> CoreResult<()> {
    match key {
        "schema_version" => fields.schema_version = Some(parse_value(value, key)?),
        "realm_id" => fields.realm_id = Some(value.to_string()),
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
        "embedding_query_template" => fields.embedding_query_template = Some(value.to_string()),
        "embedding_document_template" => {
            fields.embedding_document_template = Some(value.to_string());
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
        "visual_enabled" => fields.visual_enabled = Some(parse_value(value, key)?),
        "visual_endpoint" => fields.visual_endpoint = Some(value.to_string()),
        "visual_model" => fields.visual_model = Some(value.to_string()),
        "visual_dimensions" => fields.visual_dimensions = Some(parse_value(value, key)?),
        "visual_provider" => fields.visual_provider = Some(value.to_string()),
        "visual_revision" => fields.visual_revision = Some(value.to_string()),
        "visual_artifact_hash" => fields.visual_artifact_hash = Some(value.to_string()),
        "visual_preprocessing_version" => {
            fields.visual_preprocessing_version = Some(value.to_string());
        }
        "visual_remote_provider" => {
            fields.visual_remote_provider = Some(parse_value(value, key)?);
        }
        "visual_retention_policy" => {
            fields.visual_retention_policy = Some(value.to_string());
        }
        "sparse_enabled" => fields.sparse_enabled = Some(parse_value(value, key)?),
        "sparse_endpoint" => fields.sparse_endpoint = Some(value.to_string()),
        "sparse_provider" => fields.sparse_provider = Some(value.to_string()),
        "sparse_revision" => fields.sparse_revision = Some(value.to_string()),
        "sparse_artifact_hash" => fields.sparse_artifact_hash = Some(value.to_string()),
        "sparse_preprocessing_version" => {
            fields.sparse_preprocessing_version = Some(value.to_string());
        }
        "sparse_model" => fields.sparse_model = Some(value.to_string()),
        "sparse_vocabulary_size" => fields.sparse_vocabulary_size = Some(parse_value(value, key)?),
        "sparse_term_cap" => fields.sparse_term_cap = Some(parse_value(value, key)?),
        "sparse_remote_provider" => {
            fields.sparse_remote_provider = Some(parse_value(value, key)?);
        }
        "sparse_retention_policy" => {
            fields.sparse_retention_policy = Some(value.to_string());
        }
        other => {
            return Err(CoreError::InvalidManifest {
                key: other.to_string(),
                reason: "unknown key".to_string(),
            });
        }
    }
    Ok(())
}

fn parse_value<T>(value: &str, key: &str) -> CoreResult<T>
where
    T: std::str::FromStr,
{
    value.parse::<T>().map_err(|_| CoreError::InvalidManifest {
        key: key.to_string(),
        reason: format!("invalid value: {value}"),
    })
}
