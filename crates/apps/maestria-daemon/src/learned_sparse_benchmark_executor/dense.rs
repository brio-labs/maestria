//! Real dense projection lifecycle operations (the hybrid route's durable
//! projection when a dense provider is configured).

use anyhow::{Result, anyhow};
use maestria_domain::ContentHash;
use maestria_ports::{EmbeddingInputKind, EmbeddingProvenance, EmbeddingRequest, VectorEmbedding};
use maestria_retrieval::LearnedSparseRoute;

use maestria_retrieval::LearnedSparseOperationMeasurement;

use super::LearnedSparseBenchmarkExecutor;

impl LearnedSparseBenchmarkExecutor {
    /// Overrides the fused routes' rank fusion (benchmark configuration).
    pub fn set_fusion(
        &mut self,
        fusion: std::sync::Arc<dyn maestria_retrieval::RankFusion + Send + Sync>,
    ) {
        self.fusion = Some(fusion);
    }

    /// Lifecycle operations on the real dense projection: encode each chunk
    /// through the provider and write the vector index, mirroring
    /// `projection_recovery::reconcile_vector_projection`. Activation and
    /// rollback are the real registry transitions on the lexical generation,
    /// restored to Active before the operation ends.
    pub(super) fn dense_lifecycle_operations(
        &self,
        route: LearnedSparseRoute,
    ) -> (
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
    ) {
        let Some(index) = self.runtime.vector_index.clone() else {
            return self.empty_chunk_ops(route);
        };
        let Some(provider) = self.runtime.embedding_provider.clone() else {
            return self.empty_chunk_ops(route);
        };
        let Some(identity) = provider.identity() else {
            return self.empty_chunk_ops(route);
        };
        let Some(generation) = self.dense_generation() else {
            return self.empty_chunk_ops(route);
        };
        let model = generation.fingerprint.model.as_str().to_string();
        let chunks = self.chunks.clone();
        let one_chunk = match chunks.first() {
            Some(chunk) => chunk.id,
            None => return self.empty_chunk_ops(route),
        };
        let initial = self.measure_operation(route, "initial indexing", chunks.len(), || {
            let embeddings =
                Self::encode_embeddings(provider.as_ref(), &identity, &model, &chunks)?;
            index
                .index_embeddings(embeddings)
                .map_err(anyhow::Error::from)
        });
        let incremental = self.measure_operation(route, "incremental update", 1, || {
            let one = chunks
                .iter()
                .filter(|chunk| chunk.id == one_chunk)
                .cloned()
                .collect::<Vec<_>>();
            let embeddings = Self::encode_embeddings(provider.as_ref(), &identity, &model, &one)?;
            index
                .index_embeddings(embeddings)
                .map_err(anyhow::Error::from)
        });
        let deletion = self.measure_operation(route, "deletion", 1, || {
            index
                .delete_chunks(&[one_chunk])
                .map_err(anyhow::Error::from)
        });
        let rebuild = self.measure_operation(route, "rebuild", chunks.len(), || {
            let embeddings =
                Self::encode_embeddings(provider.as_ref(), &identity, &model, &chunks)?;
            index.rebuild(embeddings).map_err(anyhow::Error::from)
        });
        let (activation, rollback) = self.generation_transition_ops(route);
        (
            initial,
            incremental,
            deletion,
            rebuild,
            activation,
            rollback,
        )
    }

    /// The dense generation's registration, when one is active.
    fn dense_generation(&self) -> Option<&maestria_domain::IndexGeneration> {
        let id = self.runtime.dense_generation?;
        self.state.index_generations.get(id)
    }

    /// Encodes the chunks through the dense provider in stable chunk order,
    /// mirroring the daemon's vector projection recovery path.
    fn encode_embeddings(
        provider: &(dyn maestria_ports::EmbeddingProvider + Send + Sync),
        identity: &maestria_ports::EmbeddingIdentity,
        model: &str,
        chunks: &[maestria_domain::Chunk],
    ) -> Result<Vec<VectorEmbedding>> {
        chunks
            .iter()
            .map(|chunk| {
                let content_hash =
                    ContentHash::new(maestria_domain::content_hash(chunk.text.as_bytes()))?;
                let response = provider
                    .embed(EmbeddingRequest {
                        text: chunk.text.clone(),
                        model: model.to_string(),
                        kind: EmbeddingInputKind::Document,
                        identity: identity.clone(),
                    })
                    .map_err(|error| anyhow!("embed chunk {}: {error}", chunk.id))?;
                if response.identity != *identity {
                    return Err(anyhow!(
                        "embed chunk {} returned an incompatible generation identity",
                        chunk.id
                    ));
                }
                Ok(VectorEmbedding {
                    chunk_id: chunk.id,
                    vector: response.vector,
                    provenance: EmbeddingProvenance {
                        content_hash: content_hash.as_str().to_owned(),
                        identity: response.identity,
                        provider_id: response.provider_id,
                        model: response.model,
                        model_version: response.model_version,
                        disclosure: response.disclosure,
                    },
                })
            })
            .collect()
    }
}
