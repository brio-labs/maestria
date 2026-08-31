use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use maestria_domain::{
    ArtifactId, Chunk, ChunkId, DomainInput, IndexArtifactVectorsRequest, VectorIndexingCompleted,
    content_hash,
};
use maestria_governance::scan_secrets;
use maestria_ports::{EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, VectorEmbedding};
use std::sync::Arc;

impl EffectExecutionContext {
    /// Embed and project one artifact's whole pending vector chunk set
    /// (ADR-0008). The domain emits one `IndexArtifactVectors` effect per
    /// artifact generation, so this handler is total: one `embed_batch`
    /// transport call, one vector-projection upsert, one completion input.
    /// A precondition failure degrades the artifact's vector lane explicitly
    /// (stale rows invalidated) and reports `EffectFailure::Degraded`.
    pub(crate) async fn handle_index_artifact_vectors(
        &self,
        request: IndexArtifactVectorsRequest,
    ) -> Result<(), EffectFailure> {
        // Permanent per-artifact degradation: configuration cannot change
        // mid-run, so later effects short-circuit instead of repeating the
        // stale-projection invalidation.
        if let Some(reason) = self.degraded_artifact_reason(request.artifact_id) {
            tracing::debug!(
                artifact_id = %request.artifact_id,
                %reason,
                "vector indexing already degraded for artifact"
            );
            return Err(EffectFailure::Degraded(reason));
        }
        let Some(provider) = &self.adapters.embedding_provider else {
            tracing::debug!(
                artifact_id = %request.artifact_id,
                "vector indexing disabled"
            );
            let chunk_ids = self.artifact_chunk_ids(request.artifact_id).await;
            return Err(self
                .degrade_vector_with(
                    request.artifact_id,
                    &chunk_ids,
                    "embedding provider is not configured",
                )
                .await);
        };
        let (provider, model, identity) = self
            .resolve_embedding_capability(request.artifact_id, provider)
            .await?;
        let chunks = self.load_vector_chunks(request.artifact_id).await?;
        if chunks.is_empty() {
            // Everything already projected (re-driven effect after a crash
            // whose completion replayed): completion is idempotent.
            return Ok(());
        }
        let chunk_ids: Vec<ChunkId> = chunks.iter().map(|(chunk, _)| chunk.id).collect();
        let requests: Vec<EmbeddingRequest> = chunks
            .iter()
            .map(|(chunk, _)| EmbeddingRequest {
                text: chunk.text.clone(),
                model: model.clone(),
                kind: EmbeddingInputKind::Document,
                identity: identity.clone(),
            })
            .collect();
        let responses = match embed_batch_blocking(Arc::clone(&provider), requests).await {
            Ok(responses) => responses,
            Err(error) => {
                tracing::warn!(
                    artifact_id = %request.artifact_id,
                    %error,
                    "embedding provider batch failed; preserving fallback"
                );
                return self
                    .degraded_after_invalidation(&chunk_ids, "embedding provider failed")
                    .await;
            }
        };
        if responses.len() != chunks.len() {
            return self
                .degraded_after_invalidation(
                    &chunk_ids,
                    "embedding response count did not match the batch",
                )
                .await;
        }
        let embeddings = match chunks
            .iter()
            .zip(responses)
            .map(|((chunk, content_hash), response)| {
                if response.identity != identity {
                    return Err(());
                }
                Ok(VectorEmbedding {
                    chunk_id: chunk.id,
                    vector: response.vector,
                    provenance: maestria_ports::EmbeddingProvenance {
                        content_hash: content_hash.as_str().to_owned(),
                        identity: response.identity,
                        provider_id: response.provider_id,
                        model: response.model,
                        model_version: response.model_version,
                        disclosure: response.disclosure,
                    },
                })
            })
            .collect::<Result<Vec<_>, ()>>()
        {
            Ok(embeddings) => embeddings,
            Err(()) => {
                return self
                    .degraded_after_invalidation(
                        &chunk_ids,
                        "embedding response identity did not match the active generation",
                    )
                    .await;
            }
        };
        self.index_vector_embeddings(request.artifact_id, chunk_ids, embeddings)
            .await
    }

    /// Resolves the embedding provider/model/identity for an artifact,
    /// permanently degrading its vector lane when a precondition is unmet.
    async fn resolve_embedding_capability(
        &self,
        artifact_id: ArtifactId,
        provider: &Arc<dyn EmbeddingProvider + Send + Sync>,
    ) -> Result<
        (
            Arc<dyn EmbeddingProvider + Send + Sync>,
            String,
            maestria_ports::EmbeddingIdentity,
        ),
        EffectFailure,
    > {
        let artifact_chunk_ids = self.artifact_chunk_ids(artifact_id).await;
        let disclosure = provider.disclosure();
        if disclosure.remote || disclosure.retention != maestria_ports::RetentionPolicy::NoRetention
        {
            return Err(self
                .degrade_vector_with(
                    artifact_id,
                    &artifact_chunk_ids,
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
                .degrade_vector_with(
                    artifact_id,
                    &artifact_chunk_ids,
                    "embedding model is not configured",
                )
                .await);
        };
        let Some(identity) = provider.identity() else {
            return Err(self
                .degrade_vector_with(
                    artifact_id,
                    &artifact_chunk_ids,
                    "embedding provider has no generation identity",
                )
                .await);
        };
        Ok((Arc::clone(provider), model, identity))
    }

    async fn degrade_vector_with(
        &self,
        artifact_id: ArtifactId,
        chunk_ids: &[ChunkId],
        reason: &'static str,
    ) -> EffectFailure {
        tracing::warn!(artifact_id = %artifact_id, "{reason}");
        match self
            .degrade_vector_artifact(artifact_id, chunk_ids, reason)
            .await
        {
            Err(failure) => failure,
            Ok(()) => EffectFailure::Degraded(reason.to_string()),
        }
    }

    async fn index_vector_embeddings(
        &self,
        artifact_id: ArtifactId,
        chunk_ids: Vec<ChunkId>,
        embeddings: Vec<VectorEmbedding>,
    ) -> Result<(), EffectFailure> {
        let Some(vector_index) = &self.adapters.vector_index else {
            return Err(EffectFailure::Failed(
                "vector projection is unavailable: embedding capability is not configured"
                    .to_string(),
            ));
        };
        let vector_index = Arc::clone(vector_index);
        match tokio::task::spawn_blocking(move || vector_index.index_embeddings(embeddings)).await {
            Ok(Ok(())) => self.deliver_vector_completion(artifact_id),
            Ok(Err(error)) => {
                tracing::warn!(
                    artifact_id = %artifact_id,
                    %error,
                    "vector projection failed; preserving fallback"
                );
                self.degraded_after_invalidation(&chunk_ids, "vector projection failed")
                    .await
            }
            Err(error) => {
                tracing::warn!(
                    artifact_id = %artifact_id,
                    %error,
                    "vector projection task failed; preserving fallback"
                );
                self.degraded_after_invalidation(&chunk_ids, "vector projection task failed")
                    .await
            }
        }
    }

    /// Deliver the artifact-level completion input. Order-independent with
    /// the full-text lane: completion only clears pending vector state and
    /// never changes `IndexStatus`.
    fn deliver_vector_completion(&self, artifact_id: ArtifactId) -> Result<(), EffectFailure> {
        let completion = VectorIndexingCompleted { artifact_id };
        match Self::send_input(
            &self.input_tx,
            DomainInput::VectorIndexingCompleted(completion.clone()),
            "vector indexing completion",
        ) {
            Ok(()) => Ok(()),
            Err(crate::FeedbackError::CapacityFull) => {
                let input_tx = self.input_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = input_tx
                        .send(DomainInput::VectorIndexingCompleted(completion))
                        .await
                    {
                        tracing::warn!(
                            %error,
                            "vector indexing completion dropped; runtime input channel closed"
                        );
                    }
                });
                Ok(())
            }
            Err(crate::FeedbackError::RuntimeShutdown) => {
                // The vectors are already projected; only the completion
                // bookkeeping is deferred. `pending_vector_chunks` keeps the
                // artifact listed until a re-parse re-emits the effect, which
                // re-embeds nothing (rows are content-addressed by the
                // projection). Downgrade to a warning: a lost delivery at
                // shutdown is bounded bookkeeping drift, not a failed effect
                // (#486).
                tracing::warn!(
                    artifact_id = %artifact_id,
                    "vector indexing completion delivery deferred to shutdown"
                );
                Ok(())
            }
        }
    }

    /// All chunk ids of an artifact, for artifact-wide stale-row invalidation.
    async fn artifact_chunk_ids(&self, artifact_id: ArtifactId) -> Vec<ChunkId> {
        let state = self.state.read().await;
        state
            .chunks
            .values()
            .filter(|chunk| chunk.artifact_id == artifact_id)
            .map(|chunk| chunk.id)
            .collect()
    }

    /// Record permanent degradation for the artifact. Return the recorded
    /// degradation reason when this artifact's vector lane was already
    /// degraded by an earlier effect.
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
        chunk_ids: &[ChunkId],
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
        self.degraded_after_invalidation(chunk_ids, reason).await
    }

    async fn degraded_after_invalidation(
        &self,
        chunk_ids: &[ChunkId],
        reason: &'static str,
    ) -> Result<(), EffectFailure> {
        if self.adapters.vector_index.is_none() {
            tracing::debug!(
                %reason,
                "vector projection unavailable; effect degraded without invalidation"
            );
            return Err(EffectFailure::Degraded(reason.to_string()));
        }
        if self.invalidate_vector_projection(chunk_ids).await {
            Err(EffectFailure::Degraded(reason.to_string()))
        } else {
            Err(EffectFailure::Failed(format!(
                "{reason}; stale vector projection could not be invalidated"
            )))
        }
    }

    /// Load the artifact's pending vector chunks with their content hashes,
    /// applying the artifact-level retrieval and per-chunk secret checks.
    /// Any refusal degrades the artifact's whole vector lane.
    async fn load_vector_chunks(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<Vec<(Chunk, maestria_domain::ContentHash)>, EffectFailure> {
        let (chunks, artifact_chunk_ids, security_allowed) = {
            let state = self.state.read().await;
            let Some(artifact) = state.artifacts.get(&artifact_id) else {
                return Err(EffectFailure::Failed(format!(
                    "artifact {artifact_id} is missing"
                )));
            };
            let security_allowed = artifact.security.retrieval_allowed();
            let artifact_chunk_ids: Vec<ChunkId> =
                state.chunks.values().map(|chunk| chunk.id).collect();
            let chunks: Vec<Chunk> = state
                .chunks
                .values()
                .filter(|chunk| {
                    chunk.artifact_id == artifact_id
                        && state.pending_vector_chunks.contains(&chunk.id)
                })
                .cloned()
                .collect();
            (chunks, artifact_chunk_ids, security_allowed)
        };
        if !security_allowed {
            tracing::warn!(
                artifact_id = %artifact_id,
                "refusing vector indexing for denied artifact"
            );
            return Err(self
                .degrade_vector_with(
                    artifact_id,
                    &artifact_chunk_ids,
                    "artifact is not allowed for retrieval",
                )
                .await);
        }
        for chunk in &chunks {
            let secret_scan = scan_secrets(&chunk.text);
            if !secret_scan.is_clean() {
                tracing::warn!(
                    artifact_id = %artifact_id,
                    chunk_id = %chunk.id,
                    findings = secret_scan.findings.len(),
                    "refusing embedding for secret-bearing chunk"
                );
                return Err(self
                    .degrade_vector_with(
                        artifact_id,
                        &artifact_chunk_ids,
                        "chunk contains secret-like content",
                    )
                    .await);
            }
        }
        let mut resolved = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let content_hash =
                match maestria_domain::ContentHash::new(content_hash(chunk.text.as_bytes())) {
                    Ok(hash) => hash,
                    Err(_) => {
                        return Err(EffectFailure::Failed(format!(
                            "computed content hash for chunk {} is invalid",
                            chunk.id
                        )));
                    }
                };
            resolved.push((chunk, content_hash));
        }
        Ok(resolved)
    }

    async fn invalidate_vector_projection(&self, chunk_ids: &[ChunkId]) -> bool {
        if chunk_ids.is_empty() {
            return true;
        }
        let Some(vector_index) = &self.adapters.vector_index else {
            tracing::warn!("vector projection is unavailable; cannot invalidate stale rows");
            return false;
        };
        let vector_index = Arc::clone(vector_index);
        let chunk_ids = chunk_ids.to_vec();
        let result =
            tokio::task::spawn_blocking(move || vector_index.delete_chunks(&chunk_ids)).await;
        match result {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(
                    %error,
                    "could not invalidate stale vector projection"
                );
                false
            }
            Err(error) => {
                tracing::warn!(%error, "vector invalidation task failed");
                false
            }
        }
    }
}

async fn embed_batch_blocking(
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    requests: Vec<EmbeddingRequest>,
) -> Result<Vec<maestria_ports::EmbeddingResponse>, maestria_ports::PortError> {
    match tokio::task::spawn_blocking(move || provider.embed_batch(&requests)).await {
        Ok(result) => result,
        Err(error) => Err(maestria_ports::PortError::InternalContext {
            context: "embedding provider batch task failed",
            source: error.to_string(),
        }),
    }
}
