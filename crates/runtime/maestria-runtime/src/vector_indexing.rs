use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use maestria_domain::{Chunk, ChunkId, IndexVectorRequest, content_hash};
use maestria_governance::scan_secrets;
use maestria_ports::{EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, VectorEmbedding};
use std::sync::Arc;

impl EffectExecutionContext {
    pub(crate) async fn handle_index_vector(
        &self,
        request: IndexVectorRequest,
    ) -> Result<(), EffectFailure> {
        let Some(provider) = &self.adapters.embedding_provider else {
            tracing::debug!(chunk_id = %request.chunk_id, "vector indexing disabled");
            return self
                .degraded_after_invalidation(
                    request.chunk_id,
                    "embedding provider is not configured",
                )
                .await;
        };
        let Some(model) = self
            .embedding_model
            .clone()
            .filter(|model| !model.trim().is_empty())
        else {
            tracing::warn!(chunk_id = %request.chunk_id, "vector provider configured without model");
            return self
                .degraded_after_invalidation(request.chunk_id, "embedding model is not configured")
                .await;
        };
        let (chunk, content_hash) = self.load_vector_chunk(request.chunk_id).await?;
        let Some(identity) = provider.identity() else {
            tracing::warn!(chunk_id = %request.chunk_id, "embedding provider has no generation identity");
            return self
                .degraded_after_invalidation(
                    request.chunk_id,
                    "embedding provider has no generation identity",
                )
                .await;
        };
        let embedding_request = EmbeddingRequest {
            text: chunk.text.clone(),
            model,
            kind: EmbeddingInputKind::Document,
            identity: identity.clone(),
        };
        let provider = Arc::clone(provider);
        let response = match embed_blocking(provider, embedding_request).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(chunk_id = %request.chunk_id, %error, "embedding provider failed; preserving fallback");
                return self
                    .degraded_after_invalidation(request.chunk_id, "embedding provider failed")
                    .await;
            }
        };
        if response.identity != identity {
            return self
                .degraded_after_invalidation(
                    request.chunk_id,
                    "embedding response identity did not match the active generation",
                )
                .await;
        }
        let embedding = VectorEmbedding {
            chunk_id: request.chunk_id,
            vector: response.vector,
            provenance: maestria_ports::EmbeddingProvenance {
                content_hash,
                identity: response.identity,
                provider_id: response.provider_id,
                model: response.model,
                model_version: response.model_version,
                disclosure: response.disclosure,
            },
        };
        self.index_vector_embedding(request.chunk_id, embedding)
            .await
    }

    async fn index_vector_embedding(
        &self,
        chunk_id: ChunkId,
        embedding: VectorEmbedding,
    ) -> Result<(), EffectFailure> {
        let vector_index = Arc::clone(&self.adapters.vector_index);
        match tokio::task::spawn_blocking(move || vector_index.index_embeddings(vec![embedding]))
            .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "vector projection failed; preserving fallback");
                self.degraded_after_invalidation(chunk_id, "vector projection failed")
                    .await
            }
            Err(error) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "vector projection task failed; preserving fallback");
                self.degraded_after_invalidation(chunk_id, "vector projection task failed")
                    .await
            }
        }
    }

    async fn degraded_after_invalidation(
        &self,
        chunk_id: ChunkId,
        reason: &'static str,
    ) -> Result<(), EffectFailure> {
        if self.invalidate_vector_projection(chunk_id).await {
            Err(EffectFailure::Degraded(reason.to_string()))
        } else {
            Err(EffectFailure::Failed(format!(
                "{reason}; stale vector projection could not be invalidated"
            )))
        }
    }

    async fn load_vector_chunk(&self, chunk_id: ChunkId) -> Result<(Chunk, String), EffectFailure> {
        let (chunk, content_hash, security_allowed) = {
            let state = self.state.read().await;
            let Some(chunk) = state.chunks.get(&chunk_id).cloned() else {
                return Err(EffectFailure::Failed(format!(
                    "chunk {chunk_id} is missing"
                )));
            };
            let (content_hash, security_allowed) = match state.artifacts.get(&chunk.artifact_id) {
                Some(artifact) => {
                    let content_hash = match artifact.content_hash.clone() {
                        Some(content_hash) => content_hash,
                        None => content_hash(chunk.text.as_bytes()),
                    };
                    (content_hash, artifact.security.retrieval_allowed())
                }
                None => (content_hash(chunk.text.as_bytes()), false),
            };
            (chunk, content_hash, security_allowed)
        };
        if !security_allowed {
            tracing::warn!(chunk_id = %chunk_id, "refusing vector indexing for denied artifact");
            return Err(EffectFailure::Failed(
                "artifact is not allowed for retrieval".to_string(),
            ));
        }
        let secret_scan = scan_secrets(&chunk.text);
        if !secret_scan.is_clean() {
            tracing::warn!(
                chunk_id = %chunk_id,
                findings = secret_scan.findings.len(),
                "refusing embedding for secret-bearing chunk"
            );
            return Err(EffectFailure::Failed(
                "chunk contains secret-like content".to_string(),
            ));
        }
        Ok((chunk, content_hash))
    }

    async fn invalidate_vector_projection(&self, chunk_id: ChunkId) -> bool {
        let vector_index = Arc::clone(&self.adapters.vector_index);
        let result =
            tokio::task::spawn_blocking(move || vector_index.delete_chunks(&[chunk_id])).await;
        match result {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "could not invalidate stale vector projection");
                false
            }
            Err(error) => {
                tracing::warn!(chunk_id = %chunk_id, %error, "vector invalidation task failed");
                false
            }
        }
    }
}

async fn embed_blocking(
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    request: EmbeddingRequest,
) -> Result<maestria_ports::EmbeddingResponse, maestria_ports::PortError> {
    match tokio::task::spawn_blocking(move || provider.embed(request)).await {
        Ok(result) => result,
        Err(error) => Err(maestria_ports::PortError::Internal {
            message: format!("embedding provider task failed: {error}"),
        }),
    }
}
