//! Real dense projection lifecycle operations (the hybrid route's durable
//! projection when a dense provider is configured).

use anyhow::Result;
use maestria_ports::VectorEmbedding;
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
    ///
    /// Delegates to `crate::projection_recovery::embed_chunk` so the provider
    /// contract and provenance shape stay single-sourced (R28).
    fn encode_embeddings(
        provider: &(dyn maestria_ports::EmbeddingProvider + Send + Sync),
        identity: &maestria_ports::EmbeddingIdentity,
        model: &str,
        chunks: &[maestria_domain::Chunk],
    ) -> Result<Vec<VectorEmbedding>> {
        chunks
            .iter()
            .map(|chunk| crate::projection_recovery::embed_chunk(provider, identity, model, chunk))
            .collect()
    }
}
