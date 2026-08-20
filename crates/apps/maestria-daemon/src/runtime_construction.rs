use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_code_intel::{REPOSITORY_CODE_INDEX_FILENAME, RepositoryCodeIndex};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{DomainInput, KernelState};
use maestria_governance::{
    AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate, Scope,
};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_harness::LocalShellHarnessAdapter;
use maestria_parsers::ParserRegistry;
use maestria_ports::{FullTextIndex, Parser, SearchKnowledgeExecutor, VectorIndex};
use maestria_retrieval::RepositoryExecutionPolicy;
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_storage_sqlite::SqliteStore;
use maestria_web_evidence::UreqWebFetcher;
use std::{fs, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::projection_open::{
    open_base_stores, open_full_text_index, open_graph_index, open_vector_index,
    resolve_index_generations,
};
use crate::providers::build_ocr_provider;
use crate::search_executor::{
    SearchRuntime, SearchRuntimeParts, load_repository_code_index_with_exclusions,
};
use crate::vector_startup::build_embedding_provider;

#[path = "runtime_construction/policies.rs"]
mod policies;

#[cfg(test)]
pub(crate) use policies::build_sparse_retriever;
pub(crate) use policies::{hybrid_policy, learned_sparse_policy, search_lane_bundle};

struct StorageAdapters {
    blob_store: Arc<FsBlobStore>,
    sqlite_store: Arc<SqliteStore>,
}

struct IndexAdapters {
    search_index: Arc<dyn FullTextIndex + Send + Sync>,
    vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    graph_index: Arc<SqliteGraphIndex>,
}

struct EcosystemAdapters {
    parser: Arc<dyn Parser + Send + Sync>,
    ocr_provider: Option<Arc<dyn maestria_ports::OcrProvider + Send + Sync>>,
    repository_code_index: Option<Arc<RepositoryCodeIndex>>,
}

fn build_storage_adapters(layout: &InstanceLayout) -> Result<StorageAdapters> {
    let (sqlite_store, blob_store) = open_base_stores(layout)?;
    Ok(StorageAdapters {
        blob_store,
        sqlite_store,
    })
}

fn build_index_adapters(
    layout: &InstanceLayout,
    state: &KernelState,
    read_only_search_index: bool,
    has_embedding_provider: bool,
) -> Result<IndexAdapters> {
    let search_index: Arc<dyn FullTextIndex + Send + Sync> =
        open_full_text_index(layout, state, !read_only_search_index, false)?;
    let vector_index = open_vector_index(layout, has_embedding_provider)?;
    let graph_index = open_graph_index(layout, state, false)?;
    Ok(IndexAdapters {
        search_index,
        vector_index,
        graph_index,
    })
}

fn build_ecosystem_adapters(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
) -> Result<EcosystemAdapters> {
    let ocr_provider = build_ocr_provider(manifest)?;
    let parser = Arc::new(ParserRegistry::with_defaults());
    let repository_code_index =
        match load_repository_code_index_with_exclusions(layout, Some(manifest)) {
            Ok(index) => index,
            Err(error) => {
                // The repository code index is regenerable cache: a stale or
                // invalid persisted index (its repository root moved or was
                // deleted) must not block daemon startup. Remove it so the
                // next `index repository` run rebuilds it, mirroring the
                // CLI's repair-before-build path.
                let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
                tracing::warn!(
                    %error,
                    path = %index_path.display(),
                    "repository code index unhealthy; removing for rebuild"
                );
                let _ = fs::remove_file(&index_path);
                None
            }
        };
    Ok(EcosystemAdapters {
        parser,
        ocr_provider,
        repository_code_index,
    })
}

fn build_search_executor(
    state: &KernelState,
    storage: &StorageAdapters,
    indexes: &IndexAdapters,
    ecosystem: &EcosystemAdapters,
    manifest: &InstanceManifest,
    embedding_provider: Option<Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>>,
    repository_execution_policy: RepositoryExecutionPolicy,
) -> Result<Arc<dyn SearchKnowledgeExecutor + Send + Sync>> {
    let (primary_generation, corpus_snapshot, dense_generation) = resolve_index_generations(state)?;
    let (hybrid_execution_policy, learned_sparse_execution_policy, sparse_retriever) =
        crate::runtime_construction::search_lane_bundle(
            state,
            manifest,
            storage.sqlite_store.clone(),
            storage.blob_store.clone(),
        );
    let search_executor: Arc<dyn SearchKnowledgeExecutor + Send + Sync> =
        Arc::new(SearchRuntime::from_parts(
            SearchRuntimeParts {
                artifacts: storage.sqlite_store.clone(),
                cards: storage.sqlite_store.clone(),
                chunks: storage.sqlite_store.clone(),
                evidence: storage.sqlite_store.clone(),
                search_index: indexes.search_index.clone(),
                blobs: storage.blob_store.clone(),
                vector_index: indexes.vector_index.clone(),
                graph_index: Some(indexes.graph_index.clone()),
                event_log: storage.sqlite_store.clone(),
                primary_generation,
                dense_generation,
                repository_code_index: ecosystem.repository_code_index.clone(),
                repository_execution_policy,
                hybrid_execution_policy,
                learned_sparse_execution_policy,
                sparse_retriever,
                corpus_snapshot,
                scope_id: maestria_domain::DEFAULT_INSTANCE_SCOPE_ID,
            },
            embedding_provider,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )?);
    Ok(search_executor)
}

fn build_adapters(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    embedding_provider: Option<Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>>,
    repository_execution_policy: RepositoryExecutionPolicy,
    read_only_search_index: bool,
) -> Result<Adapters> {
    let storage = build_storage_adapters(layout)?;
    let indexes = build_index_adapters(
        layout,
        state,
        read_only_search_index,
        embedding_provider.is_some(),
    )?;
    let ecosystem = build_ecosystem_adapters(layout, manifest)?;
    let search_executor = build_search_executor(
        state,
        &storage,
        &indexes,
        &ecosystem,
        manifest,
        embedding_provider.clone(),
        repository_execution_policy,
    )?;
    Ok(Adapters {
        event_log: storage.sqlite_store.clone(),
        blob_store: storage.blob_store,
        search_index: indexes.search_index,
        parser: ecosystem.parser,
        ocr_provider: ecosystem.ocr_provider,
        harness: Arc::new(LocalShellHarnessAdapter),
        artifact_repo: storage.sqlite_store.clone(),
        chunk_repo: storage.sqlite_store.clone(),
        card_repo: storage.sqlite_store.clone(),
        evidence_repo: storage.sqlite_store.clone(),
        realm_read_grant_repo: storage.sqlite_store.clone(),
        embedding_provider,
        web_fetcher: Arc::new(UreqWebFetcher::new()),
        vector_index: indexes.vector_index,
        graph_index: indexes.graph_index,
        search_executor: Some(search_executor),
        id_allocator: storage.sqlite_store.clone(),
        effect_journal: storage.sqlite_store.clone(),
        approval_repo: storage.sqlite_store,
    })
}

pub(crate) fn build_runtime(
    layout: &InstanceLayout,
    state: KernelState,
    profile: AutonomyProfile,
) -> Result<(
    MaestriaRuntime,
    mpsc::Sender<DomainInput>,
    mpsc::Receiver<DomainInput>,
    CancellationToken,
)> {
    build_runtime_with_repository_policy(layout, state, profile, RepositoryExecutionPolicy::Shadow)
}

/// Build a runtime with a verified repository benchmark promotion policy.
pub(crate) fn build_runtime_with_repository_policy(
    layout: &InstanceLayout,
    state: KernelState,
    profile: AutonomyProfile,
    repository_execution_policy: RepositoryExecutionPolicy,
) -> Result<(
    MaestriaRuntime,
    mpsc::Sender<DomainInput>,
    mpsc::Receiver<DomainInput>,
    CancellationToken,
)> {
    let manifest_contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceManifest::decode(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let embedding_model = manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| config.model.clone());
    let embedding_provider = build_embedding_provider(&manifest, &state)?;
    let adapters = build_adapters(
        layout,
        &state,
        &manifest,
        embedding_provider,
        repository_execution_policy,
        false,
    )?;
    maestria_runtime::rebuild_realm_read_grant_projection(&*adapters.realm_read_grant_repo, &state)
        .with_context(|| "rebuild realm read grant projection")?;
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
        memory_promotion_gate: Arc::new(maestria_governance::DefaultMemoryPromotionGate),
    };
    let blocked_patterns = crate::blocked_patterns::runtime_blocked_patterns(&manifest);
    let scope = Scope::new(
        manifest.read_roots,
        Vec::new(),
        vec!["shell".into()],
        Vec::new(),
        false,
    )
    .with_blocked_patterns(blocked_patterns);
    let config = RuntimeConfig {
        profile,
        scope,
        embedding_model,
        ..Default::default()
    };

    let shutdown_token = CancellationToken::new();
    let (runtime, input_rx) = MaestriaRuntime::new(config, state, adapters, governance);
    let input_tx = runtime.handle().feedback_sender();
    Ok((runtime, input_tx, input_rx, shutdown_token))
}

#[cfg(test)]
#[path = "runtime_supervision_tests.rs"]
mod tests;
