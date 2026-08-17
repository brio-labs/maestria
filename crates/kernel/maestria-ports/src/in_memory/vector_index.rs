use std::sync::{Arc, Mutex};

use super::execution::{Meter, validate_limit_u32};
use crate::{
    BoundedSearch, IndexedEmbeddingKey, PortError, VectorEmbedding, VectorIndex, VectorSearchHit,
    VectorSearchQuery,
};
use maestria_domain::ChunkId;

#[derive(Clone, Default)]
pub struct InMemoryVectorIndex {
    embeddings: Arc<Mutex<Vec<VectorEmbedding>>>,
}

impl InMemoryVectorIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

fn collect_vector_hits(
    embeddings: &[VectorEmbedding],
    query: &VectorSearchQuery,
    q_norm: f64,
    filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    meter: &mut Meter,
) -> Result<
    (
        Vec<VectorSearchHit>,
        Option<maestria_domain::SearchExecutionResource>,
    ),
    PortError,
> {
    let mut hits = Vec::new();
    let mut stopped = None;
    for emb in embeddings {
        if let Some(resource) = meter.candidate() {
            stopped = Some(resource);
            break;
        }
        if query
            .provider_id
            .as_deref()
            .is_some_and(|provider| emb.provenance.provider_id != provider)
            || query
                .model
                .as_deref()
                .is_some_and(|model| emb.provenance.model != model)
            || query
                .model_version
                .as_deref()
                .is_some_and(|version| emb.provenance.model_version != version)
            || query
                .identity
                .as_ref()
                .is_some_and(|identity| emb.provenance.identity != *identity)
            || emb.vector.len() != query.vector.len()
        {
            continue;
        }
        if !filter(emb.chunk_id)? {
            continue;
        }
        let bytes = super::execution::saturating_u64(emb.vector.len()).saturating_mul(4);
        if let Some(resource) = meter.bytes(bytes) {
            stopped = Some(resource);
            break;
        }
        let work = super::execution::saturating_u64(emb.vector.len()).saturating_add(1);
        if let Some(resource) = meter.work(work) {
            stopped = Some(resource);
            break;
        }
        let mut dot = 0.0_f64;
        let mut emb_norm_sq = 0.0_f64;
        for (a, b) in emb.vector.iter().zip(&query.vector) {
            let a64 = *a as f64;
            let b64 = *b as f64;
            dot += a64 * b64;
            emb_norm_sq += a64 * a64;
        }
        let score = if emb_norm_sq == 0.0 {
            0.0
        } else {
            (dot / (emb_norm_sq.sqrt() * q_norm)) as f32
        };
        hits.push(VectorSearchHit {
            chunk_id: emb.chunk_id,
            score: if score.is_finite() { score } else { 0.0 },
        });
    }
    Ok((hits, stopped))
}

impl VectorIndex for InMemoryVectorIndex {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        for embedding in &embeddings {
            validate_vector_values(&embedding.vector, "embedding vector")?;
            if embedding.vector.len()
                != embedding.provenance.identity.fingerprint.dimensions as usize
            {
                return Err(PortError::InvalidInputContext {
                    context: "embedding vector dimension mismatch",
                    source: "vector and identity fingerprint dimensions differ".to_string(),
                });
            }
        }

        let mut guard = self
            .embeddings
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "vector index lock poisoned",
                source: "index mutex is poisoned".to_string(),
            })?;
        for emb in embeddings {
            if let Some(pos) = guard.iter().position(|e| e.chunk_id == emb.chunk_id) {
                guard[pos] = emb;
            } else {
                guard.push(emb);
            }
        }
        Ok(())
    }
    fn search_similar(
        &self,
        query: VectorSearchQuery,
    ) -> Result<BoundedSearch<VectorSearchHit>, PortError> {
        self.search_similar_filtered(query, &|_| Ok(true))
    }

    fn search_similar_filtered(
        &self,
        query: VectorSearchQuery,
        filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<VectorSearchHit>, PortError> {
        validate_vector_values(&query.vector, "query vector")?;
        validate_query_identity(&query)?;
        validate_limit_u32(
            query.limit,
            query.execution_budget,
            "vector search result limit",
        )?;
        let mut meter = Meter::new(query.execution_budget);
        if query.limit == 0 {
            return Ok(meter.complete(Vec::new()));
        }

        let guard = self
            .embeddings
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "vector index lock poisoned",
                source: "index mutex is poisoned".to_string(),
            })?;
        let q_norm_sq: f64 = query.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if q_norm_sq == 0.0 {
            return Ok(meter.complete(Vec::new()));
        }
        let q_norm = q_norm_sq.sqrt();
        let (mut hits, mut stopped) =
            collect_vector_hits(guard.as_slice(), &query, q_norm, filter, &mut meter)?;
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.chunk_id.value().cmp(&right.chunk_id.value()))
        });
        let selected = hits
            .into_iter()
            .take(super::execution::saturating_usize(u64::from(query.limit)))
            .collect::<Vec<_>>();
        for _ in 0..selected.len() {
            if let Some(resource) = meter.result() {
                stopped = Some(resource);
                break;
            }
        }
        if let Some(resource) = stopped {
            return Ok(meter.exhausted(selected, resource));
        }
        Ok(meter.complete(selected))
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        let mut guard = self
            .embeddings
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "vector index lock poisoned",
                source: "index mutex is poisoned".to_string(),
            })?;
        guard.retain(|e| !chunk_ids.contains(&e.chunk_id));
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = self
            .embeddings
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "vector index lock poisoned",
                source: "index mutex is poisoned".to_string(),
            })?;
        guard.clear();
        Ok(())
    }

    fn indexed_embedding_keys(&self) -> Result<Vec<IndexedEmbeddingKey>, PortError> {
        let guard = self
            .embeddings
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "vector index lock poisoned",
                source: "index mutex is poisoned".to_string(),
            })?;
        Ok(guard
            .iter()
            .map(|embedding| IndexedEmbeddingKey {
                chunk_id: embedding.chunk_id,
                content_hash: embedding.provenance.content_hash.clone(),
                generation_id: embedding
                    .provenance
                    .identity
                    .generation_id
                    .value()
                    .to_string(),
                representation: embedding.provenance.identity.representation.0.clone(),
                fingerprint: embedding.provenance.identity.fingerprint.encode(),
            })
            .collect())
    }

    fn reconcile_projection(
        &self,
        upserted: Vec<VectorEmbedding>,
        expected: &[ChunkId],
    ) -> Result<(), PortError> {
        self.index_embeddings(upserted)?;
        let mut guard = self
            .embeddings
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "vector index lock poisoned",
                source: "index mutex is poisoned".to_string(),
            })?;
        guard.retain(|embedding| expected.contains(&embedding.chunk_id));
        Ok(())
    }
}

fn validate_vector_values(vector: &[f32], label: &'static str) -> Result<(), PortError> {
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
fn validate_query_identity(query: &VectorSearchQuery) -> Result<(), PortError> {
    if let Some(identity) = &query.identity
        && identity.fingerprint.dimensions as usize != query.vector.len()
    {
        return Err(PortError::InvalidInputContext {
            context: "query vector dimension mismatch",
            source: "vector and identity fingerprint dimensions differ".to_string(),
        });
    }
    Ok(())
}
