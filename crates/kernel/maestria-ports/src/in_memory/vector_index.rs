use std::sync::{Arc, Mutex};

use crate::{ChunkId, PortError, VectorEmbedding, VectorIndex, VectorSearchHit, VectorSearchQuery};

#[derive(Clone, Default)]
pub struct InMemoryVectorIndex {
    embeddings: Arc<Mutex<Vec<VectorEmbedding>>>,
}

impl InMemoryVectorIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        for embedding in &embeddings {
            validate_vector_values(&embedding.vector, "embedding vector")?;
        }

        let mut guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
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

    fn search_similar(&self, query: VectorSearchQuery) -> Result<Vec<VectorSearchHit>, PortError> {
        validate_vector_values(&query.vector, "query vector")?;
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        let mut hits = Vec::new();

        let q_norm_sq: f64 = query.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if q_norm_sq == 0.0 {
            return Ok(Vec::new());
        }
        let q_norm = q_norm_sq.sqrt();

        for emb in guard.iter() {
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
                || emb.vector.len() != query.vector.len()
            {
                continue;
            }

            let mut dot: f64 = 0.0;
            let mut emb_norm_sq: f64 = 0.0;
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

            let score = if score.is_finite() { score } else { 0.0 };

            hits.push(VectorSearchHit {
                chunk_id: emb.chunk_id,
                score,
            });
        }
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.chunk_id.value().cmp(&right.chunk_id.value()))
        });
        hits.truncate(query.limit as usize);
        Ok(hits)
    }

    fn search_similar_filtered(
        &self,
        query: VectorSearchQuery,
        filter: &dyn Fn(ChunkId) -> bool,
    ) -> Result<Vec<VectorSearchHit>, PortError> {
        validate_vector_values(&query.vector, "query vector")?;
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        let mut hits = Vec::new();

        let q_norm_sq: f64 = query.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if q_norm_sq == 0.0 {
            return Ok(Vec::new());
        }
        let q_norm = q_norm_sq.sqrt();

        for emb in guard.iter() {
            if let Some(model) = &query.model
                && &emb.provenance.model != model
            {
                continue;
            }
            if let Some(version) = &query.model_version
                && &emb.provenance.model_version != version
            {
                continue;
            }
            if let Some(provider) = &query.provider_id
                && &emb.provenance.provider_id != provider
            {
                continue;
            }

            if emb.vector.len() != query.vector.len() {
                continue;
            }
            if !filter(emb.chunk_id) {
                continue;
            }

            let v_norm_sq: f64 = emb.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
            if v_norm_sq == 0.0 {
                continue;
            }
            let v_norm = v_norm_sq.sqrt();

            let dot: f64 = query
                .vector
                .iter()
                .zip(emb.vector.iter())
                .map(|(&q, &v)| (q as f64) * (v as f64))
                .sum();
            let cosine = (dot / (q_norm * v_norm)) as f32;

            hits.push(VectorSearchHit {
                chunk_id: emb.chunk_id,
                score: cosine,
            });
        }

        hits.sort_by(|left, right| {
            let order = match right.score.partial_cmp(&left.score) {
                Some(order) => order,
                None => std::cmp::Ordering::Equal,
            };
            order.then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });

        hits.truncate(query.limit as usize);
        Ok(hits)
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        let mut guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        guard.retain(|e| !chunk_ids.contains(&e.chunk_id));
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        guard.clear();
        Ok(())
    }
}

fn validate_vector_values(vector: &[f32], label: &str) -> Result<(), PortError> {
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
