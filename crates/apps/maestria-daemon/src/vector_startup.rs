use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::KernelState;
use maestria_vector_sqlite::SqliteVectorIndex;

use crate::reconcile_vector_projection;

pub(crate) fn build_embedding_provider(
    manifest: &InstanceManifest,
) -> Result<Option<Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>>> {
    manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| {
            maestria_embedding_openai::LocalHttpEmbeddingProvider::new(
                &config.endpoint,
                &config.model,
                Some(config.dimensions),
            )
            .map(|provider| {
                Arc::new(provider) as Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>
            })
            .map_err(Into::into)
        })
        .transpose()
}

/// Rebuild the vector projection for a prepared instance from replayed state.
///
/// Application entry points call this shared recovery boundary before starting
/// runtime work so missing, stale, or corrupt vector state is handled once.
pub fn reconcile_vector_projection_for_layout(
    layout: &InstanceLayout,
    state: &KernelState,
) -> Result<()> {
    let manifest_contents = std::fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceManifest::decode(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let embedding_provider = build_embedding_provider(&manifest)?;
    let embedding_model = manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| config.model.as_str());
    let vector_index = SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
        .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?;
    reconcile_vector_projection(
        state,
        &vector_index,
        embedding_provider.as_deref(),
        embedding_model,
    )
    .with_context(|| "reconcile vector projection")
}
