//! Shared store and projection opening for the search runtime and daemon runtime construction
//! paths. Both paths assemble the same store stack; the reconcile steps each path performs on
//! top of the raw opens are explicit parameters here so the two callers keep their distinct
//! recovery ownership.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{CorpusSnapshotId, IndexGenerationId, KernelState, RepresentationName};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_ports::VectorIndex;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;

/// Open the sqlite store and blob store backing every projection.
pub(crate) fn open_base_stores(
    layout: &InstanceLayout,
) -> Result<(Arc<SqliteStore>, Arc<FsBlobStore>)> {
    let sqlite_store = Arc::new(
        SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?,
    );
    let blob_store = Arc::new(
        FsBlobStore::open(&layout.blobs_dir)
            .with_context(|| format!("open blob store {}", layout.blobs_dir.display()))?,
    );
    Ok((sqlite_store, blob_store))
}

/// Open the full-text index, read-only unless writes are allowed.
///
/// When `ensure_search_index` is set and writes are allowed, the index card table is rebuilt
/// from kernel state if it is stale. The daemon runtime path performs its projection repair in
/// the lifecycle reconcile chain instead and passes `false`.
pub(crate) fn open_full_text_index(
    layout: &InstanceLayout,
    state: &KernelState,
    allow_projection_writes: bool,
    ensure_search_index: bool,
) -> Result<Arc<TantivyFullTextIndex>> {
    let search_index = if allow_projection_writes {
        Arc::new(
            TantivyFullTextIndex::open(&layout.full_text_index_dir).with_context(|| {
                format!(
                    "open full-text index {}",
                    layout.full_text_index_dir.display()
                )
            })?,
        )
    } else {
        Arc::new(
            TantivyFullTextIndex::open_read_only(&layout.full_text_index_dir).with_context(
                || {
                    format!(
                        "open full-text index read-only {}",
                        layout.full_text_index_dir.display()
                    )
                },
            )?,
        )
    };
    if ensure_search_index {
        crate::search_executor::projection::ensure_search_index(&search_index, state)?;
    }
    Ok(search_index)
}

/// Open the vector projection when an embedding provider is configured, else `None`.
pub(crate) fn open_vector_index(
    layout: &InstanceLayout,
    has_embedding_provider: bool,
) -> Result<Option<Arc<dyn VectorIndex + Send + Sync>>> {
    if !has_embedding_provider {
        return Ok(None);
    }
    Ok(Some(Arc::new(
        SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
            .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?,
    )))
}

/// Reconcile the vector projection from kernel state when writes are allowed, degrading to a
/// lexical-only search on failure.
pub(crate) fn reconcile_vector_projection(
    state: &KernelState,
    manifest: &InstanceManifest,
    embedding_provider: &Option<Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>>,
    vector_index: &Option<Arc<dyn VectorIndex + Send + Sync>>,
    allow_projection_writes: bool,
) {
    if !allow_projection_writes {
        return;
    }
    let Some(provider) = embedding_provider.as_deref() else {
        return;
    };
    let Some(vector_index) = vector_index.as_deref() else {
        return;
    };
    let model = manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| config.model.as_str());
    if let Err(error) = crate::projection_recovery::reconcile_vector_projection(
        state,
        vector_index,
        Some(provider),
        model,
    ) {
        tracing::warn!(%error, "dense retrieval unavailable; using lexical fallback");
    }
}

/// Open the graph projection, optionally reconciling it from kernel state.
///
/// The search path reconciles when projection writes are allowed; the daemon runtime path
/// leaves reconciliation to the lifecycle chain and passes `false`.
pub(crate) fn open_graph_index(
    layout: &InstanceLayout,
    state: &KernelState,
    reconcile_projection: bool,
) -> Result<Arc<SqliteGraphIndex>> {
    let graph_index = Arc::new(
        SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    if reconcile_projection {
        crate::projection_recovery::reconcile_graph_projection(state, &*graph_index)
            .with_context(|| "reconcile graph projection for search")?;
    }
    Ok(graph_index)
}

/// Resolve the active lexical and dense retrieval generations from kernel state.
pub(crate) fn resolve_index_generations(
    state: &KernelState,
) -> Result<(
    IndexGenerationId,
    CorpusSnapshotId,
    Option<IndexGenerationId>,
)> {
    let lexical = state
        .index_generations
        .get_active(&RepresentationName::new("lexical_text_v1"))
        .ok_or_else(|| anyhow!("active lexical retrieval generation is missing"))?;
    let dense_generation = state
        .index_generations
        .get_active(&RepresentationName::new("dense_text_v1"))
        .map(|generation| generation.id);
    Ok((lexical.id, lexical.corpus_snapshot, dense_generation))
}
