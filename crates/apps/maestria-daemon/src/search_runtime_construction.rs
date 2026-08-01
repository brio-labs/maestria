use super::*;
use crate::projection_open::{
    open_base_stores, open_full_text_index, open_graph_index, open_vector_index,
    reconcile_vector_projection, resolve_index_generations,
};
use anyhow::Context as _;

impl SearchRuntime {
    pub(super) fn visual_retriever(
        &self,
        active_versions: BTreeSet<ArtifactVersionId>,
    ) -> Option<Arc<dyn CandidateRetriever>> {
        let (Some(vector_index), Some(provider), Some(capability)) = (
            self.visual_vector_index.clone(),
            self.visual_embedding_provider.clone(),
            self.visual_generation.clone(),
        ) else {
            return None;
        };
        Some(Arc::new(CurrentVersionFilter::new(
            Arc::new(VisualPageRegionRetriever::new(
                VisualPageRegionRetrieverParts {
                    index: vector_index,
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                    embedding_provider: provider,
                },
                capability,
            )),
            active_versions,
        )))
    }
}

/// Construct the one search runtime used by CLI search and explain.
pub fn prepare_search_runtime(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_repository_policy(
        layout,
        state,
        manifest,
        retrieval_policy,
        RepositoryExecutionPolicy::Shadow,
    )
}

/// Construct a search runtime with a verified repository benchmark policy.
pub fn prepare_search_runtime_with_repository_policy(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    repository_execution_policy: RepositoryExecutionPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_options(
        layout,
        state,
        manifest,
        retrieval_policy,
        repository_execution_policy,
        true,
    )
}

/// Construct a search runtime without rebuilding writable projections.
pub fn prepare_search_runtime_read_only(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_read_only_with_repository_policy(
        layout,
        state,
        manifest,
        retrieval_policy,
        RepositoryExecutionPolicy::Shadow,
    )
}

/// Construct a read-only search runtime with a verified repository policy.
pub fn prepare_search_runtime_read_only_with_repository_policy(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    repository_execution_policy: RepositoryExecutionPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_options(
        layout,
        state,
        manifest,
        retrieval_policy,
        repository_execution_policy,
        false,
    )
}

fn prepare_search_runtime_with_options(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    repository_execution_policy: RepositoryExecutionPolicy,
    allow_projection_writes: bool,
) -> Result<Arc<SearchRuntime>> {
    let (sqlite_store, blob_store) = open_base_stores(layout)?;
    let search_index = open_full_text_index(
        layout,
        state,
        allow_projection_writes,
        allow_projection_writes,
    )?;
    let repository_code_index = load_repository_code_index_with_exclusions(layout, Some(manifest))
        .context("load repository code index")?;
    let embedding_provider = crate::vector_startup::build_embedding_provider(manifest, state)?;
    let vector_index = open_vector_index(layout, embedding_provider.is_some())?;
    reconcile_vector_projection(
        state,
        manifest,
        &embedding_provider,
        &vector_index,
        allow_projection_writes,
    );
    let graph_index = open_graph_index(layout, state, allow_projection_writes)?;
    let (primary_generation, corpus_snapshot, dense_generation) = resolve_index_generations(state)?;

    let parts = SearchRuntimeParts {
        artifacts: sqlite_store.clone(),
        cards: sqlite_store.clone(),
        chunks: sqlite_store.clone(),
        evidence: sqlite_store.clone(),
        search_index,
        blobs: blob_store,
        vector_index,
        event_log: sqlite_store.clone(),
        graph_index: Some(graph_index),
        primary_generation,
        dense_generation,
        repository_code_index,
        repository_execution_policy,
        corpus_snapshot,
    };
    Ok(Arc::new(SearchRuntime::from_parts(
        parts,
        embedding_provider,
        retrieval_policy,
    )?))
}
