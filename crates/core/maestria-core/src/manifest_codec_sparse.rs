use crate::error::{CoreError, CoreResult};
use maestria_ports::RetentionPolicy;
use url::Url;

use super::manifest_codec::ManifestFields;
use super::manifest_codec::common::{parse_retention_policy, string_or_empty};

fn fingerprint_field(
    enabled: bool,
    value: &Option<String>,
    missing: fn() -> CoreError,
) -> CoreResult<String> {
    if enabled {
        value.clone().ok_or_else(missing)
    } else {
        Ok(string_or_empty(value))
    }
}

fn retention_policy(fields: &ManifestFields) -> CoreResult<RetentionPolicy> {
    parse_retention_policy(
        fields
            .sparse_retention_policy
            .as_deref()
            .map_or("no_retention", |value| value),
    )
}

/// Validates the local-only activation invariants of an enabled profile.
fn validate_enabled_config(
    artifact_hash: &str,
    vocabulary_size: u32,
    term_cap: u32,
    fields: &ManifestFields,
) -> CoreResult<()> {
    maestria_domain::ContentHash::new(artifact_hash.to_string()).map_err(|error| {
        CoreError::InvalidManifest {
            key: "sparse_artifact_hash".to_string(),
            reason: format!("invalid content hash: {error}"),
        }
    })?;
    if vocabulary_size == 0 {
        return Err(CoreError::InvalidManifest {
            key: "sparse_vocabulary_size".to_string(),
            reason: "must be positive when enabled".to_string(),
        });
    }
    if term_cap == 0 || term_cap > vocabulary_size {
        return Err(CoreError::InvalidManifest {
            key: "sparse_term_cap".to_string(),
            reason: "must be within the vocabulary when enabled".to_string(),
        });
    }
    if fields.sparse_remote_provider.is_some_and(|value| value) {
        return Err(CoreError::InvalidManifest {
            key: "sparse_remote_provider".to_string(),
            reason: "must be false: sparse activation is local-only".to_string(),
        });
    }
    if retention_policy(fields)? != RetentionPolicy::NoRetention {
        return Err(CoreError::InvalidManifest {
            key: "sparse_retention_policy".to_string(),
            reason: "must be no_retention: sparse activation retains no inputs".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn parse_sparse_config(
    fields: &ManifestFields,
) -> CoreResult<Option<crate::manifest::SparseProfileConfig>> {
    match (
        &fields.sparse_enabled,
        &fields.sparse_endpoint,
        &fields.sparse_model,
    ) {
        (None, None, None) => Ok(None),
        (Some(enabled), Some(endpoint), Some(model)) => {
            validate_sparse_endpoint(endpoint)?;
            let provider =
                fingerprint_field(*enabled, &fields.sparse_provider, sparse_fingerprint_error)?;
            let revision =
                fingerprint_field(*enabled, &fields.sparse_revision, sparse_fingerprint_error)?;
            let artifact_hash = fingerprint_field(
                *enabled,
                &fields.sparse_artifact_hash,
                sparse_fingerprint_error,
            )?;
            let preprocessing_version = fingerprint_field(
                *enabled,
                &fields.sparse_preprocessing_version,
                sparse_fingerprint_error,
            )?;
            let vocabulary_size = if *enabled {
                fields
                    .sparse_vocabulary_size
                    .ok_or_else(sparse_capacity_error)?
            } else {
                0
            };
            let term_cap = if *enabled {
                fields.sparse_term_cap.ok_or_else(sparse_capacity_error)?
            } else {
                0
            };
            if *enabled {
                validate_enabled_config(&artifact_hash, vocabulary_size, term_cap, fields)?;
            }
            Ok(Some(crate::manifest::SparseProfileConfig {
                enabled: *enabled,
                endpoint: endpoint.clone(),
                provider,
                revision,
                artifact_hash,
                preprocessing_version,
                model: model.clone(),
                vocabulary_size,
                term_cap,
                remote_provider: fields.sparse_remote_provider.is_some_and(|value| value),
                retention_policy: retention_policy(fields)?,
            }))
        }
        _ => Err(CoreError::InvalidManifest {
            key: "sparse_config".to_string(),
            reason: "must define enabled, endpoint, and model".to_string(),
        }),
    }
}

fn sparse_fingerprint_error() -> CoreError {
    CoreError::InvalidManifest {
        key: "sparse_config".to_string(),
        reason: "enabled configuration requires provider, revision, artifact hash, and preprocessing version".to_string(),
    }
}

fn sparse_capacity_error() -> CoreError {
    CoreError::InvalidManifest {
        key: "sparse_config".to_string(),
        reason: "enabled configuration requires vocabulary_size and term_cap".to_string(),
    }
}

fn validate_sparse_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidManifest {
        key: "sparse_endpoint".to_string(),
        reason: format!("invalid URL: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/sparse"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidManifest {
            key: "sparse_endpoint".to_string(),
            reason: "must be an http loopback /v1/sparse URL".to_string(),
        });
    }
    Ok(())
}
