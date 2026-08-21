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

    let mut dot0 = 0.0_f64;
    let mut dot1 = 0.0_f64;
    let mut dot2 = 0.0_f64;
    let mut dot3 = 0.0_f64;
    let mut norm0 = 0.0_f64;
    let mut norm1 = 0.0_f64;
    let mut norm2 = 0.0_f64;
    let mut norm3 = 0.0_f64;

    let query_chunks = query.chunks_exact(4);
    let query_remainder = query_chunks.remainder();
    let byte_chunks = bytes.chunks_exact(4 * F32_BYTES);
    let byte_remainder = byte_chunks.remainder();

    for (q4, b16) in query_chunks.zip(byte_chunks) {
        let s0 = f32::from_le_bytes([b16[0], b16[1], b16[2], b16[3]]);
        let s1 = f32::from_le_bytes([b16[4], b16[5], b16[6], b16[7]]);
        let s2 = f32::from_le_bytes([b16[8], b16[9], b16[10], b16[11]]);
        let s3 = f32::from_le_bytes([b16[12], b16[13], b16[14], b16[15]]);

        if !s0.is_finite() || !s1.is_finite() || !s2.is_finite() || !s3.is_finite() {
            return Err(PortError::InternalContext {
                context: "stored vector blob contains non-finite value",
                source: "decoded value is not finite".to_string(),
            });
        }

        let l0 = q4[0] as f64;
        let r0 = s0 as f64;
        dot0 += l0 * r0;
        norm0 += r0 * r0;

        let l1 = q4[1] as f64;
        let r1 = s1 as f64;
        dot1 += l1 * r1;
        norm1 += r1 * r1;

        let l2 = q4[2] as f64;
        let r2 = s2 as f64;
        dot2 += l2 * r2;
        norm2 += r2 * r2;

        let l3 = q4[3] as f64;
        let r3 = s3 as f64;
        dot3 += l3 * r3;
        norm3 += r3 * r3;
    }

    for (query_value, chunk) in query_remainder
        .iter()
        .zip(byte_remainder.chunks_exact(F32_BYTES))
    {
        let stored = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !stored.is_finite() {
            return Err(PortError::InternalContext {
                context: "stored vector blob contains non-finite value",
                source: "decoded value is not finite".to_string(),
            });
        }
        let l = *query_value as f64;
        let r = stored as f64;
        dot0 += l * r;
        norm0 += r * r;
    }

    let dot = (dot0 + dot1) + (dot2 + dot3);
    let right_norm = (norm0 + norm1) + (norm2 + norm3);

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
