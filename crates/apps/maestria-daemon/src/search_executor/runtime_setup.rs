use anyhow::Context;
use maestria_domain::KernelState;

use super::*;

impl SearchRuntime {
    #[cfg(test)]
    pub(crate) fn from_parts(
        parts: SearchRuntimeParts,
        embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Result<Self> {
        Self::from_parts_with_source_manifest(
            parts,
            embedding_provider,
            retrieval_policy,
            None,
            None,
        )
    }

    pub(crate) fn from_parts_with_manifest(
        parts: SearchRuntimeParts,
        embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
        source_manifest: Arc<RwLock<InstanceManifest>>,
        source_layout: InstanceLayout,
    ) -> Result<Self> {
        Self::from_parts_with_source_manifest(
            parts,
            embedding_provider,
            retrieval_policy,
            Some(source_manifest),
            Some(source_layout),
        )
    }

    fn from_parts_with_source_manifest(
        parts: SearchRuntimeParts,
        embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
        source_manifest: Option<Arc<RwLock<InstanceManifest>>>,
        source_layout: Option<InstanceLayout>,
    ) -> Result<Self> {
        let fingerprint =
            RetrievalModelFingerprint::new("maestria-core:deterministic-v1".to_string())
                .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self {
            artifacts: parts.artifacts,
            cards: parts.cards,
            chunks: parts.chunks,
            evidence: parts.evidence,
            search_index: parts.search_index,
            blobs: parts.blobs,
            vector_index: parts.vector_index,
            visual_vector_index: None,
            graph_index: parts.graph_index,
            event_log: parts.event_log,
            persist_learned_sparse_observations: true,
            embedding_provider,
            reranker: None,
            visual_embedding_provider: None,
            visual_generation: None,
            retrieval_policy,
            primary_generation: parts.primary_generation,
            dense_generation: parts.dense_generation,
            repository_code_index: parts.repository_code_index,
            repository_execution_policy: parts.repository_execution_policy,
            hybrid_execution_policy: parts.hybrid_execution_policy,
            visual_execution_policy: VisualExecutionPolicy::Shadow,
            learned_sparse_execution_policy: parts.learned_sparse_execution_policy,
            sparse_retriever: parts.sparse_retriever,
            corpus_snapshot: parts.corpus_snapshot,
            scope_id: parts.scope_id,
            fingerprint,
            interactive_search_workers: Arc::new(tokio::sync::Semaphore::new(2)),
            engine_cache: Arc::new(RwLock::new(None)),
            interactive_cache: Arc::new(RwLock::new(None)),
            source_manifest,
            source_layout,
            allowed_roots: None,
        })
    }

    pub(crate) fn assemble(
        layout: &InstanceLayout,
        state: &KernelState,
        manifest: &InstanceManifest,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
        repository_execution_policy: RepositoryExecutionPolicy,
        allow_projection_writes: bool,
        federation_read_only: bool,
    ) -> Result<Arc<Self>> {
        use crate::projection_open::{
            open_base_stores, open_base_stores_read_only, open_full_text_index, open_graph_index,
            open_vector_index, reconcile_vector_projection, resolve_index_generations,
        };
        let (sqlite_store, blob_store) = if federation_read_only {
            open_base_stores_read_only(layout)?
        } else {
            open_base_stores(layout)?
        };
        let search_index = open_full_text_index(
            layout,
            state,
            allow_projection_writes,
            allow_projection_writes,
        )?;
        let repository_code_index =
            crate::search_executor::load_repository_code_index_with_exclusions(
                layout,
                Some(manifest),
            )
            .context("load repository code index")?;
        let embedding_provider = if federation_read_only {
            None
        } else {
            crate::vector_startup::build_embedding_provider(manifest, state)?
        };
        let vector_index = if federation_read_only {
            None
        } else {
            open_vector_index(layout, embedding_provider.is_some())?
        };
        reconcile_vector_projection(
            state,
            manifest,
            &embedding_provider,
            &vector_index,
            allow_projection_writes,
        );
        let graph_index: Option<Arc<dyn GraphIndex + Send + Sync>> = if federation_read_only {
            None
        } else {
            Some(open_graph_index(layout, state, allow_projection_writes)?)
        };
        let (primary_generation, corpus_snapshot, dense_generation) =
            resolve_index_generations(state)?;
        let (hybrid_execution_policy, learned_sparse_execution_policy, sparse_retriever) =
            crate::runtime_construction::search_lane_bundle(
                state,
                manifest,
                sqlite_store.clone(),
                blob_store.clone(),
            );
        let parts = SearchRuntimeParts {
            artifacts: sqlite_store.clone(),
            cards: sqlite_store.clone(),
            chunks: sqlite_store.clone(),
            evidence: sqlite_store.clone(),
            search_index,
            blobs: blob_store,
            vector_index,
            graph_index,
            event_log: sqlite_store,
            primary_generation,
            dense_generation,
            repository_code_index,
            repository_execution_policy,
            hybrid_execution_policy,
            learned_sparse_execution_policy,
            sparse_retriever,
            corpus_snapshot,
            scope_id: maestria_domain::DEFAULT_INSTANCE_SCOPE_ID,
        };
        Ok(Arc::new(Self::from_parts_with_manifest(
            parts,
            embedding_provider,
            retrieval_policy,
            Arc::new(RwLock::new(manifest.clone())),
            layout.clone(),
        )?))
    }
}
