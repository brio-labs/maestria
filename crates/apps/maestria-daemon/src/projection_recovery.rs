use anyhow::{Context, Result};
use maestria_domain::KernelState;
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EmbeddingProvider, EmbeddingRequest,
    EvidenceRepository, GraphIndex, VectorEmbedding, VectorIndex,
};
use maestria_storage_sqlite::SqliteStore;

/// Reconcile projection repositories from replayed domain truth.
///
/// After `load_kernel_state` replays the event log, this helper idempotently upserts every artifact,
/// chunk, and card, and unconditionally replaces every evidence row from the replayed state into
/// the SQLite projection tables. Evidence uses `replace` so a valid replayed row overwrites a
/// stale, malformed, or partial row from a prior crash without tripping a `Conflict` error.
///
/// Projection repair never emits domain events and never changes event truth. Startup recovery can
/// then search/open evidence even if the previous process crashed after event append but before a
/// projection write.
pub fn reconcile_projections(state: &KernelState, store: &SqliteStore) -> Result<()> {
    for artifact in state.artifacts.values() {
        ArtifactRepository::put(store, artifact.clone())
            .with_context(|| format!("put artifact {}", artifact.id))?;
    }
    for chunk in state.chunks.values() {
        ChunkRepository::put(store, chunk.clone())
            .with_context(|| format!("put chunk {}", chunk.id))?;
    }
    for card in state.cards.values() {
        CardRepository::put(store, card.clone())
            .with_context(|| format!("put card {}", card.id))?;
    }
    for evidence in state.evidences.values() {
        EvidenceRepository::replace(store, evidence.clone())
            .with_context(|| format!("replace evidence {}", evidence.id))?;
    }
    Ok(())
}

/// Rebuild the graph projection from replayed, evidenced relations.
///
/// Graph storage is a disposable projection. Clearing and rebuilding it at
/// startup repairs rows lost after an event was appended but before its graph
/// effect completed, while unevidenced relations remain intentionally absent.
pub fn reconcile_graph_projection(state: &KernelState, graph: &impl GraphIndex) -> Result<()> {
    let relations = state
        .relations
        .values()
        .filter(|relation| relation.evidence_id.is_some())
        .cloned()
        .collect();
    graph
        .rebuild(relations)
        .context("rebuild graph projection from domain state")?;
    Ok(())
}
/// Rebuild the vector projection from replayed chunks and the configured
/// embedding provider.
///
/// Vector rows are disposable and never determine domain truth. When
/// embeddings are disabled, rebuilding with an empty set removes stale rows.
/// When embeddings are enabled, every replayed chunk is embedded in stable
/// `ChunkId` order and the provider response supplies its provenance.
pub fn reconcile_vector_projection(
    state: &KernelState,
    vector_index: &(dyn VectorIndex + Send + Sync),
    embedding_provider: Option<&(dyn EmbeddingProvider + Send + Sync)>,
    embedding_model: Option<&str>,
) -> Result<()> {
    let embeddings = match (embedding_provider, embedding_model) {
        (None, None) => Vec::new(),
        (Some(provider), Some(model)) if !model.trim().is_empty() => state
            .chunks
            .values()
            .map(|chunk| {
                let content_hash = match state
                    .artifacts
                    .get(&chunk.artifact_id)
                    .and_then(|artifact| artifact.content_hash.clone())
                {
                    Some(content_hash) => content_hash,
                    None => maestria_domain::content_hash(chunk.text.as_bytes()),
                };
                let response = provider
                    .embed(EmbeddingRequest {
                        text: chunk.text.clone(),
                        model: model.to_string(),
                    })
                    .map_err(|error| anyhow::anyhow!("embed chunk {}: {error}", chunk.id))?;
                Ok(VectorEmbedding {
                    chunk_id: chunk.id,
                    vector: response.vector,
                    provenance: maestria_ports::EmbeddingProvenance {
                        content_hash,
                        provider_id: response.provider_id,
                        model: response.model,
                        model_version: response.model_version,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?,
        (Some(_), Some(_)) => {
            return Err(anyhow::anyhow!(
                "vector projection recovery requires a non-empty embedding model"
            ));
        }
        (Some(_), None) => {
            return Err(anyhow::anyhow!(
                "vector projection recovery has an embedding provider but no model"
            ));
        }
        (None, Some(_)) => {
            return Err(anyhow::anyhow!(
                "vector projection recovery has an embedding model but no provider"
            ));
        }
    };
    vector_index
        .rebuild(embeddings)
        .context("rebuild vector projection from domain state")?;
    Ok(())
}
