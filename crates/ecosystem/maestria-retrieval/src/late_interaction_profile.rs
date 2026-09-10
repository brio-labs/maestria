use std::path::Path;

use maestria_domain::{
    ContentHash, CorpusSnapshotId, FingerprintRevision, IndexFingerprint, IndexGeneration,
    IndexGenerationId, ModelName, PreprocessingVersion, ProviderName, QuantizationScheme, RealmId,
    RepresentationName, TrustZone,
};
use maestria_ports::{
    MULTIVECTOR_TEXT_V1, MultiVectorCompression, MultiVectorFingerprint, MultiVectorIdentity,
    MultiVectorSimilarity, MultiVectorTokenPolicy,
};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LateInteractionProfileError {
    #[error("read late-interaction profile: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse late-interaction profile: {0}")]
    Json(#[from] serde_json::Error),
    #[error("late-interaction profile field {0} is missing")]
    Missing(&'static str),
    #[error("late-interaction profile field {field} is invalid: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("late-interaction generation identity mismatch: {0}")]
    GenerationMismatch(String),
    #[error("late-interaction profile identity is invalid: {0}")]
    Identity(#[from] maestria_ports::PortError),
}
fn required<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a Value, LateInteractionProfileError> {
    let mut current = value;
    for segment in field.split('.') {
        current = current
            .get(segment)
            .ok_or(LateInteractionProfileError::Missing(field))?;
    }
    Ok(current)
}

fn text(value: &Value, field: &'static str) -> Result<String, LateInteractionProfileError> {
    required(value, field)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| LateInteractionProfileError::Invalid {
            field,
            reason: "expected string".to_owned(),
        })
}

fn bool_value(value: &Value, field: &'static str) -> Result<bool, LateInteractionProfileError> {
    required(value, field)?
        .as_bool()
        .ok_or_else(|| LateInteractionProfileError::Invalid {
            field,
            reason: "expected boolean".to_owned(),
        })
}

fn u32_value(value: &Value, field: &'static str) -> Result<u32, LateInteractionProfileError> {
    let number =
        required(value, field)?
            .as_u64()
            .ok_or_else(|| LateInteractionProfileError::Invalid {
                field,
                reason: "expected non-negative integer".to_owned(),
            })?;
    u32::try_from(number).map_err(|_| LateInteractionProfileError::Invalid {
        field,
        reason: "integer exceeds u32".to_owned(),
    })
}

fn hash(value: &Value, field: &'static str) -> Result<ContentHash, LateInteractionProfileError> {
    ContentHash::new(text(value, field)?).map_err(|error| LateInteractionProfileError::Invalid {
        field,
        reason: error.to_string(),
    })
}

fn enum_value<T>(
    value: &Value,
    field: &'static str,
    expected: &str,
    output: T,
) -> Result<T, LateInteractionProfileError> {
    let actual = text(value, field)?;
    if actual != expected {
        return Err(LateInteractionProfileError::Invalid {
            field,
            reason: format!("expected {expected}, got {actual}"),
        });
    }
    Ok(output)
}
fn policy(
    root: &Value,
    native_token_limit: &'static str,
    max_vectors: &'static str,
    max_utf8_bytes: &'static str,
    prefix_id: &'static str,
    prefix_text: &'static str,
) -> Result<MultiVectorTokenPolicy, LateInteractionProfileError> {
    Ok(MultiVectorTokenPolicy {
        prefix_id: u32_value(root, prefix_id)?.into(),
        prefix_text: text(root, prefix_text)?,
        native_token_limit: u32_value(root, native_token_limit)?,
        max_vectors: u32_value(root, max_vectors)?,
        max_utf8_bytes: u32_value(root, max_utf8_bytes)?,
        truncate_right_after_prefix: enum_value(
            root,
            "truncate_direction",
            "right_after_prefix",
            true,
        )?,
        retain_special_tokens: bool_value(root, "retain_special_tokens")?,
        drop_masked_positions: bool_value(root, "drop_masked_positions")?,
        pad_token_id: u32_value(root, "pad_token_id")?.into(),
        query_expansion: bool_value(root, "query_expansion")?,
        lowercase: bool_value(root, "lowercase")?,
        punctuation_pruning: bool_value(root, "punctuation_pruning")?,
    })
}
fn exact_text(
    value: &Value,
    field: &'static str,
    expected: &str,
) -> Result<(), LateInteractionProfileError> {
    let actual = text(value, field)?;
    if actual != expected {
        return Err(LateInteractionProfileError::Invalid {
            field,
            reason: format!("expected {expected}, got {actual}"),
        });
    }
    Ok(())
}

fn exact_u32(
    value: &Value,
    field: &'static str,
    expected: u32,
) -> Result<(), LateInteractionProfileError> {
    let actual = u32_value(value, field)?;
    if actual != expected {
        return Err(LateInteractionProfileError::Invalid {
            field,
            reason: format!("expected {expected}, got {actual}"),
        });
    }
    Ok(())
}

fn validate_profile_runtime(root: &Value) -> Result<(), LateInteractionProfileError> {
    required(root, "limits")?;
    required(root, "query_prefix")?;
    required(root, "document_prefix")?;
    exact_u32(root, "profile_schema_version", 1)?;
    exact_text(root, "fingerprint_algorithm", "late-interaction-profile-v1")?;
    exact_u32(root, "runtime.inference_workers", 1)?;
    exact_u32(root, "runtime.intra_op_threads", 2)?;
    let providers = required(root, "onnx_providers")?.as_array().ok_or(
        LateInteractionProfileError::Invalid {
            field: "onnx_providers",
            reason: "expected array".to_owned(),
        },
    )?;
    if providers.len() != 1 || providers[0].as_str() != Some("CPUExecutionProvider") {
        return Err(LateInteractionProfileError::Invalid {
            field: "onnx_providers",
            reason: "only CPUExecutionProvider is supported".to_owned(),
        });
    }
    Ok(())
}

fn load_fingerprint(root: &Value) -> Result<MultiVectorFingerprint, LateInteractionProfileError> {
    let limits = required(root, "limits")?;
    let base = IndexFingerprint {
        provider: ProviderName::new(text(root, "provider")?),
        model: ModelName::new(text(root, "model")?),
        revision: FingerprintRevision::new(text(root, "revision")?),
        artifact_hash: hash(root, "artifact_hash")?,
        dimensions: u32_value(root, "output_dimensions")?,
        quantization: QuantizationScheme::new(text(root, "weight_quantization")?),
        query_template_hash: hash(root, "query_template_hash")?,
        document_template_hash: hash(root, "document_template_hash")?,
        preprocessing_version: PreprocessingVersion::new(text(root, "preprocessing_version")?),
    };
    exact_u32(root, "model_config.embedding_dim", base.dimensions)?;
    exact_u32(root, "model_config.native_query_token_limit", 8192)?;
    exact_u32(root, "model_config.native_document_token_limit", 8192)?;
    exact_u32(
        root,
        "model_config.pad_token_id",
        u32_value(root, "pad_token_id")?,
    )?;
    exact_u32(
        root,
        "model_config.mask_token_id",
        u32_value(root, "mask_token_id")?,
    )?;
    let query_policy = policy(
        root,
        "native_query_token_limit",
        "query_max_vectors",
        "limits.query_utf8_bytes",
        "query_prefix.id",
        "query_prefix.text",
    )?;
    let document_policy = policy(
        root,
        "native_document_token_limit",
        "document_max_vectors",
        "limits.document_utf8_bytes",
        "document_prefix.id",
        "document_prefix.text",
    )?;
    let fingerprint = MultiVectorFingerprint {
        base,
        tokenizer_hash: hash(root, "tokenizer_hash")?,
        vocabulary_hash: hash(root, "vocabulary_hash")?,
        vocabulary_size: u32_value(root, "vocabulary_size")?,
        similarity: enum_value(root, "similarity", "Cosine", MultiVectorSimilarity::Cosine)?,
        aggregation: enum_value(
            root,
            "aggregation",
            "QueryTokenMaxThenSum",
            maestria_domain::LateInteractionAggregation::QueryTokenMaxThenSum,
        )?,
        normalization_version: text(root, "normalization")?,
        representation_compression: enum_value(
            root,
            "representation_compression",
            "none",
            MultiVectorCompression::None,
        )?,
        query_policy,
        document_policy,
    };
    if u32_value(limits, "query_utf8_bytes")? != fingerprint.query_policy.max_utf8_bytes
        || u32_value(limits, "document_utf8_bytes")? != fingerprint.document_policy.max_utf8_bytes
    {
        return Err(LateInteractionProfileError::Invalid {
            field: "limits",
            reason: "text byte limits do not match token policies".to_owned(),
        });
    }
    Ok(fingerprint)
}
/// Loads the frozen profile and binds it to the live generation/corpus/realm identity.
///
/// The profile is intentionally parsed as a JSON value so the loader can reject
/// absent or malformed identity fields while allowing non-identity operational
/// metadata to evolve independently. Every field that contributes to the
/// canonical multivector identity is required and compared to the expected
/// frozen profile values.
pub fn load_late_interaction_identity(
    path: impl AsRef<Path>,
    generation_id: IndexGenerationId,
    corpus_snapshot: CorpusSnapshotId,
    realm: RealmId,
    trust_zone: TrustZone,
) -> Result<MultiVectorIdentity, LateInteractionProfileError> {
    let bytes = std::fs::read(path)?;
    let root: Value = serde_json::from_slice(&bytes)?;
    validate_profile_runtime(&root)?;
    let identity = MultiVectorIdentity {
        representation: RepresentationName::new(MULTIVECTOR_TEXT_V1),
        fingerprint: load_fingerprint(&root)?,
        generation_id,
        corpus_snapshot,
        realm,
        trust_zone,
    };
    identity.validate()?;
    Ok(identity)
}

/// Binds a loaded profile to the exact live generation before provider use.
pub fn validate_late_interaction_generation(
    generation: &IndexGeneration,
    identity: &MultiVectorIdentity,
) -> Result<(), LateInteractionProfileError> {
    if generation.id != identity.generation_id {
        return Err(LateInteractionProfileError::GenerationMismatch(
            "generation ID differs from profile identity".to_owned(),
        ));
    }
    if generation.corpus_snapshot != identity.corpus_snapshot {
        return Err(LateInteractionProfileError::GenerationMismatch(
            "corpus snapshot differs from profile identity".to_owned(),
        ));
    }
    if generation.fingerprint != identity.fingerprint.base {
        return Err(LateInteractionProfileError::GenerationMismatch(
            "base fingerprint differs from profile identity".to_owned(),
        ));
    }
    if generation.name.as_str() != MULTIVECTOR_TEXT_V1 {
        return Err(LateInteractionProfileError::GenerationMismatch(
            "generation representation is not multivector_text_v1".to_owned(),
        ));
    }
    let digest = identity.digest()?;
    generation
        .validate_representation_fingerprint(&digest)
        .map_err(|error| LateInteractionProfileError::GenerationMismatch(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::IndexLifecycle;

    fn profile_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/contracts/late_interaction_profile_v1.json")
    }
    #[test]
    fn loads_frozen_profile_into_identity() -> Result<(), Box<dyn std::error::Error>> {
        let path = profile_path();
        let identity = load_late_interaction_identity(
            path,
            IndexGenerationId::new(1),
            CorpusSnapshotId::new(1),
            RealmId::try_from("a".repeat(64))?,
            TrustZone::Verified,
        )?;
        assert_eq!(identity.fingerprint.base.dimensions, 128);
        assert_eq!(identity.fingerprint.query_policy.max_vectors, 128);
        assert_eq!(identity.fingerprint.document_policy.max_vectors, 512);
        assert!(identity.digest().is_ok());
        Ok(())
    }

    #[test]
    fn binds_profile_to_generation_digest() -> Result<(), Box<dyn std::error::Error>> {
        let path = profile_path();
        let identity = load_late_interaction_identity(
            path,
            IndexGenerationId::new(7),
            CorpusSnapshotId::new(9),
            RealmId::try_from("b".repeat(64))?,
            TrustZone::Verified,
        )?;
        let generation = IndexGeneration {
            id: identity.generation_id,
            name: RepresentationName::new(MULTIVECTOR_TEXT_V1),
            corpus_snapshot: identity.corpus_snapshot,
            representation_fingerprint: Some(identity.digest()?),
            sparse_namespace: None,
            fingerprint: identity.fingerprint.base.clone(),
            lifecycle: IndexLifecycle::Active,
        };
        validate_late_interaction_generation(&generation, &identity)?;
        let mut drifted = generation;
        drifted.representation_fingerprint =
            Some(ContentHash::new(format!("sha256:{}", "c".repeat(64)))?);
        assert!(validate_late_interaction_generation(&drifted, &identity).is_err());
        Ok(())
    }
}
