use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use maestria_domain::{ArtifactId, Chunk, ChunkId, IndexChunkRequest, content_hash};
use maestria_governance::scan_secrets;
use maestria_ports::{EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, VectorEmbedding};
use std::sync::Arc;

impl EffectExecutionContext {
    pub(crate) async fn handle_index_vector(
        &self,
        request: IndexChunkRequest,
    ) -> Result<(), EffectFailure> {
        // Permanent per-artifact degradation: configuration cannot change
        // mid-run, so later chunks short-circuit instead of repeating the
        // per-chunk stale-projection invalidation.
        if let Some(reason) = self.degraded_artifact_reason(request.artifact_id) {
            tracing::debug!(
                artifact_id = %request.artifact_id,
                chunk_id = %request.chunk_id,
                %reason,
                "vector indexing already degraded for artifact"
            );
            return Err(EffectFailure::Degraded(reason));
        }
        let Some(provider) = &self.adapters.embedding_provider else {
            tracing::debug!(chunk_id = %request.chunk_id, "vector indexing disabled");
            return self
                .degrade_vector_artifact(
                    request.artifact_id,
                    request.chunk_id,
                    "embedding provider is not configured",
                )
                .await;
        };
        let (provider, model, identity) = self
            .resolve_embedding_capability(&request, provider)
            .await?;
        let (chunk, content_hash) = self.load_vector_chunk(request.chunk_id).await?;
        let embedding_request = EmbeddingRequest {
            text: chunk.text.clone(),
            model,
            kind: EmbeddingInputKind::Document,
            identity: identity.clone(),
        };
        let response = match embed_blocking(provider, embedding_request).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    chunk_id = %request.chunk_id,
                    %error,
                    "embedding provider failed; preserving fallback"
                );
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
                content_hash: content_hash.as_str().to_owned(),
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

    /// Resolves the embedding provider/model/identity for a chunk request,
    /// permanently degrading the artifact when a precondition is unmet.
    async fn resolve_embedding_capability(
        &self,
        request: &IndexChunkRequest,
        provider: &Arc<dyn EmbeddingProvider + Send + Sync>,
    ) -> Result<
        (
            Arc<dyn EmbeddingProvider + Send + Sync>,
            String,
            maestria_ports::EmbeddingIdentity,
        ),
        EffectFailure,
    > {
        let disclosure = provider.disclosure();
        if disclosure.remote || disclosure.retention != maestria_ports::RetentionPolicy::NoRetention
        {
            return Err(self
                .degrade_vector_with(
                    request,
                    "embedding transport violates local no-retention policy",
                )
                .await);
        }
        let Some(model) = self
            .embedding_model
            .clone()
            .filter(|model| !model.trim().is_empty())
        else {
            return Err(self
                .degrade_vector_with(request, "embedding model is not configured")
                .await);
        };
        let Some(identity) = provider.identity() else {
            return Err(self
                .degrade_vector_with(request, "embedding provider has no generation identity")
                .await);
        };
        Ok((Arc::clone(provider), model, identity))
    }

    async fn degrade_vector_with(
        &self,
        request: &IndexChunkRequest,
        reason: &'static str,
    ) -> EffectFailure {
        tracing::warn!(chunk_id = %request.chunk_id, "{reason}");
        match self
            .degrade_vector_artifact(request.artifact_id, request.chunk_id, reason)
            .await
        {
            Err(failure) => failure,
            Ok(()) => EffectFailure::Degraded(reason.to_string()),
        }
    }

    async fn index_vector_embedding(
        &self,
        chunk_id: ChunkId,
        embedding: VectorEmbedding,
    ) -> Result<(), EffectFailure> {
        let Some(vector_index) = &self.adapters.vector_index else {
            return Err(EffectFailure::Failed(
                "vector projection is unavailable: embedding capability is not configured"
                    .to_string(),
            ));
        };
        let vector_index = Arc::clone(vector_index);
        match tokio::task::spawn_blocking(move || vector_index.index_embeddings(vec![embedding]))
            .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                tracing::warn!(
                    chunk_id = %chunk_id,
                    %error,
                    "vector projection failed; preserving fallback"
                );
                self.degraded_after_invalidation(chunk_id, "vector projection failed")
                    .await
            }
            Err(error) => {
                tracing::warn!(
                    chunk_id = %chunk_id,
                    %error,
                    "vector projection task failed; preserving fallback"
                );
                self.degraded_after_invalidation(chunk_id, "vector projection task failed")
                    .await
            }
        }
    }

    /// Record permanent degradation for the artifact, running the
    /// stale-projection invalidation only for its first degraded chunk.
    /// Return the recorded degradation reason when this artifact's vector
    fn degraded_artifact_reason(&self, artifact_id: ArtifactId) -> Option<String> {
        let degraded = match self.degraded_vector_artifacts.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("degraded vector artifacts lock poisoned on read");
                poisoned.into_inner()
            }
        };
        degraded.get(&artifact_id).cloned()
    }

    async fn degrade_vector_artifact(
        &self,
        artifact_id: ArtifactId,
        chunk_id: ChunkId,
        reason: &'static str,
    ) -> Result<(), EffectFailure> {
        let first_for_artifact = match self.degraded_vector_artifacts.lock() {
            Ok(mut degraded) => match degraded.entry(artifact_id) {
                std::collections::btree_map::Entry::Occupied(_) => false,
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(reason.to_string());
                    true
                }
            },
            // A poisoned lock only means the artifact already degraded once;
            // skip the invalidation and stay degraded.
            Err(_) => false,
        };
        if !first_for_artifact {
            return Err(EffectFailure::Degraded(reason.to_string()));
        }
        self.degraded_after_invalidation(chunk_id, reason).await
    }

    async fn degraded_after_invalidation(
        &self,
        chunk_id: ChunkId,
        reason: &'static str,
    ) -> Result<(), EffectFailure> {
        if self.adapters.vector_index.is_none() {
            tracing::debug!(
                chunk_id = %chunk_id,
                %reason,
                "vector projection unavailable; effect degraded without invalidation"
            );
            return Err(EffectFailure::Degraded(reason.to_string()));
        }
        if self.invalidate_vector_projection(chunk_id).await {
            Err(EffectFailure::Degraded(reason.to_string()))
        } else {
            Err(EffectFailure::Failed(format!(
                "{reason}; stale vector projection could not be invalidated"
            )))
        }
    }

    async fn load_vector_chunk(
        &self,
        chunk_id: ChunkId,
    ) -> Result<(Chunk, maestria_domain::ContentHash), EffectFailure> {
        let (chunk, content_hash, security_allowed) = {
            let state = self.state.read().await;
            let Some(chunk) = state.chunks.get(&chunk_id).cloned() else {
                return Err(EffectFailure::Failed(format!(
                    "chunk {chunk_id} is missing"
                )));
            };
            // The embedded-content identity is the chunk text hash; it must
            // match the identity every projection writer stores, or the
            // startup reconcile skip re-embeds the whole corpus.
            let content_hash =
                match maestria_domain::ContentHash::new(content_hash(chunk.text.as_bytes())) {
                    Ok(hash) => hash,
                    Err(_) => {
                        return Err(EffectFailure::Failed(format!(
                            "computed content hash for chunk {chunk_id} is invalid"
                        )));
                    }
                };
            let security_allowed = state
                .artifacts
                .get(&chunk.artifact_id)
                .is_some_and(|artifact| artifact.security.retrieval_allowed());
            (chunk, content_hash, security_allowed)
        };
        if !security_allowed {
            tracing::warn!(chunk_id = %chunk_id, "refusing vector indexing for denied artifact");
            return self
                .degrade_or_fail(
                    chunk.artifact_id,
                    chunk_id,
                    "artifact is not allowed for retrieval",
                )
                .await;
        }
        let secret_scan = scan_secrets(&chunk.text);
        if !secret_scan.is_clean() {
            tracing::warn!(
                chunk_id = %chunk_id,
                findings = secret_scan.findings.len(),
                "refusing embedding for secret-bearing chunk"
            );
            return self
                .degrade_or_fail(
                    chunk.artifact_id,
                    chunk_id,
                    "chunk contains secret-like content",
                )
                .await;
        }
        Ok((chunk, content_hash))
    }

    /// Degrade the vector lane for the artifact, propagating the resulting
    /// failure; never returns `Ok` (a poisoned degradation lock still
    /// degrades), so the caller can propagate the `EffectFailure` directly.
    async fn degrade_or_fail(
        &self,
        artifact_id: ArtifactId,
        chunk_id: ChunkId,
        reason: &'static str,
    ) -> Result<(Chunk, maestria_domain::ContentHash), EffectFailure> {
        Err(
            match self
                .degrade_vector_artifact(artifact_id, chunk_id, reason)
                .await
            {
                Err(failure) => failure,
                Ok(()) => EffectFailure::Degraded(reason.to_string()),
            },
        )
    }

    async fn invalidate_vector_projection(&self, chunk_id: ChunkId) -> bool {
        let Some(vector_index) = &self.adapters.vector_index else {
            tracing::warn!(
                chunk_id = %chunk_id,
                "vector projection is unavailable; cannot invalidate stale rows"
            );
            return false;
        };
        let vector_index = Arc::clone(vector_index);
        let result =
            tokio::task::spawn_blocking(move || vector_index.delete_chunks(&[chunk_id])).await;
        match result {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(
                    chunk_id = %chunk_id,
                    %error,
                    "could not invalidate stale vector projection"
                );
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
        Err(error) => Err(maestria_ports::PortError::InternalContext {
            context: "embedding provider task failed",
            source: error.to_string(),
        }),
    }
}
