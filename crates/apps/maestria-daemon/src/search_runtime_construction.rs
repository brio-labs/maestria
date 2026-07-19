use super::*;

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
                self.retrieval_policy.clone(),
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
    let sqlite_store = Arc::new(
        SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?,
    );
    let blob_store = Arc::new(
        FsBlobStore::open(&layout.blobs_dir)
            .with_context(|| format!("open blob store {}", layout.blobs_dir.display()))?,
    );
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
    if allow_projection_writes {
        super::projection::ensure_search_index(&search_index, state)?;
    }
    let repository_code_index = load_repository_code_index_with_exclusions(layout, Some(manifest))
        .context("load repository code index")?;
    let embedding_provider = crate::build_embedding_provider(manifest, state)?;
    let vector_index: Option<Arc<dyn VectorIndex + Send + Sync>> = if embedding_provider.is_some() {
        Some(Arc::new(
            SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db")).with_context(
                || format!("open vector index {}", layout.vector_index_dir.display()),
            )?,
        ))
    } else {
        None
    };
    if allow_projection_writes
        && let (Some(provider), Some(vector_index)) =
            (embedding_provider.as_deref(), vector_index.as_deref())
    {
        let model = manifest
            .embeddings
            .as_ref()
            .filter(|config| config.enabled)
            .map(|config| config.model.as_str());
        if let Err(error) =
            crate::reconcile_vector_projection(state, vector_index, Some(provider), model)
        {
            eprintln!("dense retrieval unavailable; using lexical fallback: {error}");
        }
    }
    let graph_index = Arc::new(
        SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    if allow_projection_writes {
        crate::reconcile_graph_projection(state, &*graph_index)
            .with_context(|| "reconcile graph projection for search")?;
    }
    let lexical = state
        .index_generations
        .get_active(&RepresentationName::new("lexical_text_v1"))
        .ok_or_else(|| anyhow!("active lexical retrieval generation is missing"))?;
    let dense_generation = state
        .index_generations
        .get_active(&RepresentationName::new("dense_text_v1"))
        .map(|generation| generation.id);
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
        primary_generation: lexical.id,
        dense_generation,
        repository_code_index,
        repository_execution_policy,
        corpus_snapshot: lexical.corpus_snapshot,
    };
    Ok(Arc::new(SearchRuntime::from_parts(
        parts,
        embedding_provider,
        retrieval_policy,
    )?))
}
