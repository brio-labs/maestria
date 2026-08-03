use std::mem::size_of;

use maestria_domain::ChunkId;
use maestria_domain::IndexFingerprint;
use maestria_ports::{PortError, VectorEmbedding};

pub(crate) const F32_BYTES: usize = size_of::<f32>();

pub(crate) struct PreparedEmbedding {
    pub(crate) chunk_id: ChunkId,
    pub(crate) dimension: usize,
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_hash: String,
    pub(crate) provider_id: String,
    pub(crate) model: String,
    pub(crate) model_version: String,
    pub(crate) generation_id: String,
    pub(crate) representation: String,
    pub(crate) fingerprint: String,
    pub(crate) disclosure_remote: bool,
    pub(crate) retention_policy: String,
}

impl TryFrom<VectorEmbedding> for PreparedEmbedding {
    type Error = PortError;

    fn try_from(embedding: VectorEmbedding) -> Result<Self, Self::Error> {
        validate_vector(&embedding.vector, "embedding vector")?;
        if embedding.provenance.content_hash.is_empty()
            || embedding.provenance.provider_id.is_empty()
            || embedding.provenance.model.is_empty()
            || embedding.provenance.model_version.is_empty()
        {
            return Err(PortError::InvalidInputContext {
                context: "embedding provenance fields are empty",
                source: "content hash, provider ID, model, or model version".to_string(),
            });
        }
        let dimension = embedding.vector.len();
        if dimension != embedding.provenance.identity.fingerprint.dimensions as usize {
            return Err(PortError::InvalidInputContext {
                context: "embedding vector dimension mismatch",
                source: "vector and identity fingerprint dimensions differ".to_string(),
            });
        }
        let bytes = encode_vector(&embedding.vector)?;
        Ok(Self {
            chunk_id: embedding.chunk_id,
            dimension,
            bytes,
            content_hash: embedding.provenance.content_hash,
            provider_id: embedding.provenance.provider_id,
            model: embedding.provenance.model,
            model_version: embedding.provenance.model_version,
            generation_id: embedding
                .provenance
                .identity
                .generation_id
                .value()
                .to_string(),
            disclosure_remote: embedding.provenance.disclosure.remote,
            retention_policy: match embedding.provenance.disclosure.retention {
                maestria_ports::RetentionPolicy::NoRetention => "no_retention".to_string(),
                maestria_ports::RetentionPolicy::ProviderDefined => "provider_defined".to_string(),
            },
            representation: embedding.provenance.identity.representation.0.clone(),
            fingerprint: serialize_fingerprint(&embedding.provenance.identity.fingerprint),
        })
    }
}

pub(crate) fn serialize_fingerprint(f: &IndexFingerprint) -> String {
    let mut serialized = String::new();
    let mut append = |value: &str| {
        serialized.push_str(&value.len().to_string());
        serialized.push(':');
        serialized.push_str(value);
    };
    let dimensions = f.dimensions.to_string();
    append(f.provider.as_str());
    append(f.model.as_str());
    append(f.revision.as_str());
    append(f.artifact_hash.as_str());
    append(&dimensions);
    append(f.quantization.as_str());
    append(f.query_template_hash.as_str());
    append(f.document_template_hash.as_str());
    append(f.preprocessing_version.as_str());
    serialized
}

pub(crate) fn validate_vector(vector: &[f32], label: &str) -> Result<(), PortError> {
    if vector.is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "vector is empty",
            source: label.to_string(),
        });
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(PortError::InvalidInputContext {
            context: "vector contains non-finite values",
            source: label.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn encode_vector(vector: &[f32]) -> Result<Vec<u8>, PortError> {
    let capacity =
        vector
            .len()
            .checked_mul(F32_BYTES)
            .ok_or_else(|| PortError::InvalidInputContext {
                context: "embedding vector is too large",
                source: "byte capacity overflow".to_string(),
            })?;
    let mut bytes = Vec::with_capacity(capacity);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>, PortError> {
    if !bytes.len().is_multiple_of(F32_BYTES) {
        return Err(PortError::InternalContext {
            context: "stored vector blob has invalid length",
            source: "byte length is not divisible by f32 width".to_string(),
        });
    }

    let mut vector = Vec::with_capacity(bytes.len() / F32_BYTES);
    for chunk in bytes.chunks_exact(F32_BYTES) {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !value.is_finite() {
            return Err(PortError::InternalContext {
                context: "stored vector blob contains non-finite value",
                source: "decoded value is not finite".to_string(),
            });
        }
        vector.push(value);
    }
    Ok(vector)
}

pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32, PortError> {
    if left.len() != right.len() {
        return Err(PortError::InternalContext {
            context: "stored vector dimension mismatch",
            source: "stored and query dimensions differ".to_string(),
        });
    }

    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        let l = *left_value as f64;
        let r = *right_value as f64;
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return Ok(0.0);
    }

    let score = (dot / (left_norm.sqrt() * right_norm.sqrt())) as f32;
    Ok(if score.is_finite() { score } else { 0.0 })
}

pub(crate) fn u64_to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInputContext {
        context: "id exceeds sqlite integer range",
        source: value.to_string(),
    })
}

pub(crate) fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::InternalContext {
        context: "stored id is negative",
        source: value.to_string(),
    })
}

pub(crate) fn usize_to_i64(value: usize) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInputContext {
        context: "dimension exceeds sqlite integer range",
        source: value.to_string(),
    })
}

pub(crate) fn to_port_error(error: rusqlite::Error) -> PortError {
    PortError::InternalContext {
        context: "sqlite vector projection error",
        source: error.to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::serialize_fingerprint;

    #[test]
    fn fingerprint_serialization_is_collision_free_for_delimiters()
    -> Result<(), Box<dyn std::error::Error>> {
        let base =
            maestria_ports::contract_tests::fixture_embedding_identity("model", 2)?.fingerprint;
        let mut first = base.clone();
        first.provider = "a:b".to_string().into();
        first.model = "c".to_string().into();
        let mut second = base;
        second.provider = "a".to_string().into();
        second.model = "b:c".to_string().into();
        assert_ne!(
            serialize_fingerprint(&first),
            serialize_fingerprint(&second)
        );
        Ok(())
    }
}
