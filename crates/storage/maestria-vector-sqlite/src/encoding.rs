use std::mem::size_of;

use maestria_domain::ChunkId;
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
            fingerprint: embedding.provenance.identity.fingerprint.encode(),
        })
    }
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
    let capacity = vector.len().checked_mul(F32_BYTES).ok_or_else(|| {
        PortError::invalid_input("embedding vector is too large", "byte capacity overflow")
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn cosine_similarity_bytes(
    query: &[f32],
    query_norm_sqrt: f64,
    bytes: &[u8],
) -> Result<f32, PortError> {
    if !bytes.len().is_multiple_of(F32_BYTES) {
        return Err(PortError::InternalContext {
            context: "stored vector blob has invalid length",
            source: "byte length is not divisible by f32 width".to_string(),
        });
    }
    if bytes.len() / F32_BYTES != query.len() {
        return Err(PortError::InternalContext {
            context: "stored vector dimension mismatch",
            source: "stored and query dimensions differ".to_string(),
        });
    }
    if query_norm_sqrt == 0.0 {
        return Ok(0.0);
    }

    let mut dot = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (query_value, chunk) in query.iter().zip(bytes.chunks_exact(F32_BYTES)) {
        let stored = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !stored.is_finite() {
            return Err(PortError::InternalContext {
                context: "stored vector blob contains non-finite value",
                source: "decoded value is not finite".to_string(),
            });
        }
        let l = *query_value as f64;
        let r = stored as f64;
        dot += l * r;
        right_norm += r * r;
    }

    if right_norm == 0.0 {
        return Ok(0.0);
    }

    let score = (dot / (query_norm_sqrt * right_norm.sqrt())) as f32;
    Ok(if score.is_finite() { score } else { 0.0 })
}

pub(crate) use maestria_sqlite_support::{i64_to_u64, to_port_error, u64_to_i64, usize_to_i64};
#[cfg(test)]
mod tests {
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
        assert_ne!(first.encode(), second.encode());
        Ok(())
    }
}
