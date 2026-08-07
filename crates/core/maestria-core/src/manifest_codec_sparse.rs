use super::{CoreError, CoreResult, ManifestFields, parse_retention_policy, string_or_empty};
use maestria_ports::RetentionPolicy;
use url::Url;

pub(crate) fn parse_sparse_config(
    fields: &ManifestFields,
) -> CoreResult<Option<super::super::SparseProfileConfig>> {
    match (&fields.sparse_enabled, &fields.sparse_endpoint, &fields.sparse_model) {
        (None, None, None) => Ok(None),
        (Some(enabled), Some(endpoint), Some(model)) => {
            validate_sparse_endpoint(endpoint)?;
            let provider = if *enabled {
                fields
                    .sparse_provider
                    .clone()
                    .ok_or_else(sparse_fingerprint_error)?
            } else {
                string_or_empty(&fields.sparse_provider)
            };
            let revision = if *enabled {
                fields
                    .sparse_revision
                    .clone()
                    .ok_or_else(sparse_fingerprint_error)?
            } else {
                string_or_empty(&fields.sparse_revision)
            };
            let artifact_hash = if *enabled {
                fields
                    .sparse_artifact_hash
                    .clone()
                    .ok_or_else(sparse_fingerprint_error)?
            } else {
                string_or_empty(&fields.sparse_artifact_hash)
            };
            let preprocessing_version = if *enabled {
                fields
                    .sparse_preprocessing_version
                    .clone()
                    .ok_or_else(sparse_fingerprint_error)?
            } else {
                string_or_empty(&fields.sparse_preprocessing_version)
            };
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
                maestria_domain::ContentHash::new(artifact_hash.clone()).map_err(|error| {
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
                let retention_policy = parse_retention_policy(
                    fields
                        .sparse_retention_policy
                        .as_deref()
                        .map_or("no_retention", |value| value),
                )?;
                if retention_policy != RetentionPolicy::NoRetention {
                    return Err(CoreError::InvalidManifest {
                        key: "sparse_retention_policy".to_string(),
                        reason: "must be no_retention: sparse activation retains no inputs"
                            .to_string(),
                    });
                }
            }
            Ok(Some(super::super::SparseProfileConfig {
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
                retention_policy: parse_retention_policy(
                    fields
                        .sparse_retention_policy
                        .as_deref()
                        .map_or("no_retention", |value| value),
                )?,
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
