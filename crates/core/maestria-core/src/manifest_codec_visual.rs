use super::{CoreError, CoreResult, ManifestFields, parse_retention_policy, string_or_empty};
use url::Url;

pub(crate) fn parse_visual_config(
    fields: &ManifestFields,
) -> CoreResult<Option<super::super::VisualConfig>> {
    match (
        &fields.visual_enabled,
        &fields.visual_endpoint,
        &fields.visual_model,
        &fields.visual_dimensions,
    ) {
        (None, None, None, None) => Ok(None),
        (Some(enabled), Some(endpoint), Some(model), Some(dimensions)) => {
            validate_visual_endpoint(endpoint)?;
            if *enabled && *dimensions == 0 {
                return Err(CoreError::InvalidInput {
                    message: "visual_dimensions must be positive when enabled".to_string(),
                });
            }
            let provider = if *enabled {
                fields
                    .visual_provider
                    .clone()
                    .ok_or_else(visual_fingerprint_error)?
            } else {
                string_or_empty(&fields.visual_provider)
            };
            let revision = if *enabled {
                fields
                    .visual_revision
                    .clone()
                    .ok_or_else(visual_fingerprint_error)?
            } else {
                string_or_empty(&fields.visual_revision)
            };
            let artifact_hash = if *enabled {
                fields
                    .visual_artifact_hash
                    .clone()
                    .ok_or_else(visual_fingerprint_error)?
            } else {
                string_or_empty(&fields.visual_artifact_hash)
            };
            let preprocessing_version = if *enabled {
                fields
                    .visual_preprocessing_version
                    .clone()
                    .ok_or_else(visual_fingerprint_error)?
            } else {
                string_or_empty(&fields.visual_preprocessing_version)
            };
            if *enabled {
                maestria_domain::ContentHash::new(artifact_hash.clone()).map_err(|error| {
                    CoreError::InvalidInput {
                        message: format!("invalid visual artifact hash: {error}"),
                    }
                })?;
            }
            Ok(Some(super::super::VisualConfig {
                enabled: *enabled,
                endpoint: endpoint.clone(),
                model: model.clone(),
                dimensions: *dimensions,
                provider,
                revision,
                artifact_hash,
                preprocessing_version,
                remote_provider: fields.visual_remote_provider.is_some_and(|value| value),
                retention_policy: parse_retention_policy(
                    fields
                        .visual_retention_policy
                        .as_deref()
                        .map_or("no_retention", |value| value),
                )?,
            }))
        }
        _ => Err(CoreError::InvalidInput {
            message: "visual configuration must define enabled, endpoint, model, and dimensions"
                .to_string(),
        }),
    }
}

fn visual_fingerprint_error() -> CoreError {
    CoreError::InvalidInput {
        message:
            "enabled visual configuration requires provider, revision, artifact hash, and preprocessing version"
                .to_string(),
    }
}

fn validate_visual_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidInput {
        message: format!("invalid visual endpoint: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/embeddings"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidInput {
            message: "visual endpoint must be an http loopback /v1/embeddings URL".to_string(),
        });
    }
    Ok(())
}
