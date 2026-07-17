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
        let dimension = embedding.vector.len();
        if dimension != embedding.provenance.identity.fingerprint.dimensions as usize {
            return Err(PortError::InvalidInput {
                message: "embedding vector dimension does not match its identity fingerprint"
                    .into(),
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
    append(&f.provider);
    append(&f.model);
    append(&f.revision);
    append(f.artifact_hash.as_str());
    append(&dimensions);
    append(&f.quantization);
    append(&f.query_template_hash);
    append(&f.document_template_hash);
    append(&f.preprocessing_version);
    serialized
}

pub(crate) fn validate_vector(vector: &[f32], label: &str) -> Result<(), PortError> {
    if vector.is_empty() {
        return Err(PortError::InvalidInput {
            message: format!("{label} must not be empty"),
        });
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(PortError::InvalidInput {
            message: format!("{label} must contain only finite values"),
        });
    }
    Ok(())
}

pub(crate) fn encode_vector(vector: &[f32]) -> Result<Vec<u8>, PortError> {
    let capacity = vector
        .len()
        .checked_mul(F32_BYTES)
        .ok_or_else(|| PortError::InvalidInput {
            message: "embedding vector is too large".to_string(),
        })?;
    let mut bytes = Vec::with_capacity(capacity);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>, PortError> {
    if !bytes.len().is_multiple_of(F32_BYTES) {
        return Err(PortError::Internal {
            message: "stored vector blob has invalid length".to_string(),
        });
    }

    let mut vector = Vec::with_capacity(bytes.len() / F32_BYTES);
    for chunk in bytes.chunks_exact(F32_BYTES) {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !value.is_finite() {
            return Err(PortError::Internal {
                message: "stored vector blob contains non-finite value".to_string(),
            });
        }
        vector.push(value);
    }
    Ok(vector)
}

pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32, PortError> {
    if left.len() != right.len() {
        return Err(PortError::Internal {
            message: "stored vector dimension does not match query vector".to_string(),
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
    i64::try_from(value).map_err(|_| PortError::InvalidInput {
        message: format!("id {value} exceeds sqlite integer range"),
    })
}

pub(crate) fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::Internal {
        message: format!("stored id {value} is negative"),
    })
}

pub(crate) fn usize_to_i64(value: usize) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInput {
        message: format!("dimension {value} exceeds sqlite integer range"),
    })
}

pub(crate) fn to_port_error(error: rusqlite::Error) -> PortError {
    PortError::Internal {
        message: format!("sqlite vector projection error: {error}"),
    }
}
#[cfg(test)]
mod tests {
    use super::serialize_fingerprint;
    use maestria_ports::EmbeddingIdentity;

    #[test]
    fn fingerprint_serialization_is_collision_free_for_delimiters()
    -> Result<(), Box<dyn std::error::Error>> {
        let base = EmbeddingIdentity::legacy("model", 2)?.fingerprint;
        let mut first = base.clone();
        first.provider = "a:b".to_string();
        first.model = "c".to_string();
        let mut second = base;
        second.provider = "a".to_string();
        second.model = "b:c".to_string();
        assert_ne!(
            serialize_fingerprint(&first),
            serialize_fingerprint(&second)
        );
        Ok(())
    }
}
