pub mod api;

/// Responsibility map:
/// - `api`: module responsibility.
/// - `lock`: module responsibility.
/// - `search_executor`: module responsibility.
/// - `approval_recovery`: module responsibility.
/// - `projection_recovery`: module responsibility.
/// - `vector_startup`: module responsibility.
/// - `full_text_recovery`: module responsibility.
/// - `parser_resume`: module responsibility.
/// - `recovery_inputs`: module responsibility.
/// - `supervision_recovery`: module responsibility.
/// - `validation_recovery`: module responsibility.
/// - `lifecycle`: module responsibility.
/// - `watcher`: module responsibility.
pub use api::{ApiServer, ClientOperation, ClientRequest, ClientResponse, DaemonClient};

mod lock;
pub use lock::{
    InstanceWriteLock, acquire as acquire_instance_write_lock,
    try_acquire as try_acquire_instance_write_lock,
};
mod search_executor;

use std::{fs, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceManifest, InstanceService};
#[cfg(test)]
use maestria_domain::ArtifactId;
use maestria_domain::{DomainInput, KernelState, RepresentationName, replay_events};
use maestria_governance::{
    AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate,
    PrivacyExclusions, Scope,
};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_harness::LocalShellHarnessAdapter;
use maestria_ocr_local::LocalHttpOcrProvider;
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    EmbeddingIdentity, EventFilter, FullTextIndex, OcrIdentity, OcrProvider,
    SearchKnowledgeExecutor, VectorIndex, VisualEmbeddingProvider,
};
use maestria_retrieval::RepositoryExecutionPolicy;
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
use maestria_visual_local::LocalHttpVisualProvider;
use maestria_web_evidence::UreqWebFetcher;
pub use search_executor::{
    SearchRuntime, prepare_search_runtime, prepare_search_runtime_read_only,
    prepare_search_runtime_read_only_with_repository_policy,
    prepare_search_runtime_with_repository_policy,
};
use search_executor::{SearchRuntimeParts, load_repository_code_index_with_exclusions};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

mod approval_recovery;
mod projection_recovery;
pub use approval_recovery::{reconcile_approval_repo, reconcile_pending_approvals};
pub use projection_recovery::{
    reconcile_graph_projection, reconcile_projections, reconcile_vector_projection,
};
mod vector_startup;
pub use vector_startup::{
    RetrievalGenerations, build_embedding_provider, reconcile_retrieval_generations,
    reconcile_vector_projection_for_layout,
};
mod full_text_recovery;
pub use full_text_recovery::pending_start_full_text;
mod parser_resume;
pub use parser_resume::verify_pending_blobs;
mod recovery_inputs;
pub use recovery_inputs::{RecoveryInputs, recovery_inputs};
mod supervision_recovery;
pub use supervision_recovery::{RecoveryDiagnostics, supervise_recovery};
mod validation_recovery;
pub use validation_recovery::has_current_validation_report;
mod lifecycle;
mod watcher;
pub use lifecycle::{InstanceLifecycle, RecoveryQueue, run_instance, run_instance_with_shutdown};
/// Validate that every pending `ResumeParser` source path is within the
/// instance manifest read scope before the daemon touches blobs or runtime
/// infrastructure.  Out-of-scope and excluded pending parsers fail fast
/// with a descriptive error, avoiding useless blob reads and runtime work.
pub fn validate_recovery_scope(layout: &InstanceLayout, recovery: &RecoveryInputs) -> Result<()> {
    let manifest_contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceManifest::decode(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest for recovery scope: {error}"))?;
    let privacy = PrivacyExclusions::default();

    for input in &recovery.resume_parsers {
        if let DomainInput::ResumeParser(record) = input {
            let source = std::path::Path::new(&record.source_path);
            if !manifest.allows_source(source) {
                return Err(anyhow!(
                    "resume parser source path is outside the instance manifest read scope \
                     or excluded by pattern: {} (artifact {} \"{}\")",
                    record.source_path,
                    record.artifact_id,
                    record.title,
                ));
            }
            if privacy.is_excluded(source) {
                return Err(anyhow!(
                    "resume parser source path is excluded by privacy policy: \
                     {} (artifact {} \"{}\")",
                    record.source_path,
                    record.artifact_id,
                    record.title,
                ));
            }
        }
    }
    Ok(())
}

/// Reconcile the approval repository from replayed domain events.
///
/// After `load_kernel_state` replays the event log, this function scans for
/// `ApprovalRecorded` events and ensures the approval repository reflects the
/// resolved state. If a CLI-initiated resolution persisted the event but crashed
/// before updating the repo, this repair brings the repo back into consistency.
pub fn prepare_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let plan = InstanceService::init_instance(InitInstanceInput { root: instance_dir })?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    if !plan.manifest_path.exists() {
        fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    }
    Ok(plan.layout)
}

pub fn load_kernel_state(layout: &InstanceLayout) -> Result<KernelState> {
    let sqlite_store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let events =
        maestria_ports::EventLog::scan(&sqlite_store, EventFilter { artifact_id: None })
            .with_context(|| format!("scan domain events {}", layout.database_path.display()))?;
    replay_events(&events).map_err(|error| anyhow!(error))
}
fn build_ocr_provider(manifest: &InstanceManifest) -> Result<Option<Arc<dyn OcrProvider>>> {
    let Some(config) = manifest.ocr.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    let identity = OcrIdentity {
        provider: config.provider.clone(),
        model: config.model.clone(),
        revision: config.revision.clone(),
        artifact_hash: config.artifact_hash.clone(),
        preprocessing_version: config.preprocessing_version.clone(),
    };
    let provider = LocalHttpOcrProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local OCR provider: {error}"))?;
    Ok(Some(Arc::new(provider)))
}

/// Builds the configured visual provider for an active visual generation.
///
/// The generation identity is supplied by the caller so model vectors cannot
/// be used before the corresponding visual index generation is activated.
pub fn build_visual_provider(
    manifest: &InstanceManifest,
    identity: EmbeddingIdentity,
) -> Result<Option<Arc<dyn VisualEmbeddingProvider + Send + Sync>>> {
    let Some(config) = manifest.visual.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    if config.remote_provider
        || !matches!(
            config.retention_policy,
            maestria_ports::RetentionPolicy::NoRetention
        )
    {
        return Err(anyhow!(
            "visual provider must be local and no-retention before activation"
        ));
    }
    if identity.fingerprint.model != config.model
        || identity.fingerprint.dimensions != config.dimensions as u32
        || identity.fingerprint.provider != config.provider
        || identity.fingerprint.revision != config.revision
        || identity.fingerprint.preprocessing_version != config.preprocessing_version
        || identity.fingerprint.artifact_hash.to_string() != config.artifact_hash
    {
        return Err(anyhow!(
            "visual provider configuration does not match active generation identity"
        ));
    }
    let provider = LocalHttpVisualProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local visual provider: {error}"))?;
    Ok(Some(Arc::new(provider)))
}

/// Reports visual capability without touching the model endpoint.
pub fn visual_status(manifest: &InstanceManifest) -> Result<String> {
    let Some(config) = manifest.visual.as_ref() else {
        return Ok("disabled (no visual configuration)".to_string());
    };
    if !config.enabled {
        return Ok("disabled (visual_enabled=false)".to_string());
    }
    if config.remote_provider
        || !matches!(
            config.retention_policy,
            maestria_ports::RetentionPolicy::NoRetention
        )
    {
        return Ok(format!(
            "configured but rejected (provider={} model={} requires local no-retention)",
            config.provider, config.model
        ));
    }
    Ok(format!(
        "configured local provider={} model={} endpoint={} activation=requires-fingerprinted-visual-generation",
        config.provider, config.model, config.endpoint
    ))
}

pub fn ocr_status(manifest: &InstanceManifest) -> Result<String> {
    let Some(config) = manifest.ocr.as_ref() else {
        return Ok("disabled (no ocr configuration)".to_string());
    };
    if !config.enabled {
        return Ok("disabled (ocr_enabled=false)".to_string());
    }
    let identity = OcrIdentity {
        provider: config.provider.clone(),
        model: config.model.clone(),
        revision: config.revision.clone(),
        artifact_hash: config.artifact_hash.clone(),
        preprocessing_version: config.preprocessing_version.clone(),
    };
    let provider = LocalHttpOcrProvider::new(&config.endpoint, &config.model, identity)
        .map_err(|error| anyhow!("configure local OCR provider: {error}"))?;
    match provider.check_local_tools() {
        Ok(()) => Ok(format!(
            "configured local provider={} model={} endpoint={} rasterizer=ready",
            config.provider, config.model, config.endpoint
        )),
        Err(error) => Ok(format!(
            "configured local provider={} model={} endpoint={} rasterizer=unavailable: {}",
            config.provider, config.model, config.endpoint, error
        )),
    }
}
fn build_adapters(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    embedding_provider: Option<Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>>,
    repository_execution_policy: RepositoryExecutionPolicy,
    read_only_search_index: bool,
) -> Result<Adapters> {
    let blob_store = Arc::new(
        FsBlobStore::open(&layout.blobs_dir)
            .with_context(|| format!("open blob store {}", layout.blobs_dir.display()))?,
    );
    let search_index: Arc<dyn FullTextIndex + Send + Sync> = if read_only_search_index {
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
    } else {
        Arc::new(
            TantivyFullTextIndex::open(&layout.full_text_index_dir).with_context(|| {
                format!(
                    "open full-text index {}",
                    layout.full_text_index_dir.display()
                )
            })?,
        )
    };
    let parser = Arc::new(ParserRegistry::with_optional_ocr(build_ocr_provider(
        manifest,
    )?));
    let sqlite_store = Arc::new(
        SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?,
    );
    let vector_index: Arc<dyn VectorIndex + Send + Sync> = Arc::new(
        SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
            .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?,
    );
    let graph_index = Arc::new(
        SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    let lexical = state
        .index_generations
        .get_active(&RepresentationName::new("lexical_text_v1"))
        .ok_or_else(|| anyhow!("active lexical retrieval generation is missing"))?;
    let dense_generation = state
        .index_generations
        .get_active(&RepresentationName::new("dense_text_v1"))
        .map(|generation| generation.id);
    let repository_code_index = load_repository_code_index_with_exclusions(layout, Some(manifest))
        .with_context(|| "load repository code index for runtime construction")?;
    let search_executor: Arc<dyn SearchKnowledgeExecutor + Send + Sync> =
        Arc::new(SearchRuntime::from_parts(
            SearchRuntimeParts {
                artifacts: sqlite_store.clone(),
                cards: sqlite_store.clone(),
                chunks: sqlite_store.clone(),
                evidence: sqlite_store.clone(),
                search_index: search_index.clone(),
                blobs: blob_store.clone(),
                vector_index: Some(vector_index.clone()),
                graph_index: Some(graph_index.clone()),
                event_log: sqlite_store.clone(),
                primary_generation: lexical.id,
                dense_generation,
                repository_code_index,
                repository_execution_policy,
                corpus_snapshot: lexical.corpus_snapshot,
            },
            embedding_provider.clone(),
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )?);
    Ok(Adapters {
        event_log: sqlite_store.clone(),
        blob_store,
        search_index,
        parser,
        harness: Arc::new(LocalShellHarnessAdapter),
        artifact_repo: sqlite_store.clone(),
        chunk_repo: sqlite_store.clone(),
        card_repo: sqlite_store.clone(),
        evidence_repo: sqlite_store.clone(),
        embedding_provider,
        web_fetcher: Arc::new(UreqWebFetcher::new()),
        vector_index,
        graph_index,
        search_executor: Some(search_executor),
        id_allocator: sqlite_store.clone(),
        effect_journal: sqlite_store.clone(),
        approval_repo: sqlite_store,
    })
}

pub fn build_runtime(
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
pub fn build_runtime_with_repository_policy(
    layout: &InstanceLayout,
    mut state: KernelState,
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
    let manifest = InstanceService::parse_manifest(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    reconcile_retrieval_generations(layout, &mut state, &manifest)
        .context("reconcile retrieval generations before runtime construction")?;
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
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
        memory_promotion_gate: Arc::new(maestria_governance::DefaultMemoryPromotionGate),
    };
    let default_privacy = PrivacyExclusions::default();
    let mut blocked_patterns = manifest.excluded_patterns.clone();
    blocked_patterns.extend(default_privacy.sensitive_names().iter().cloned());
    blocked_patterns.extend(
        default_privacy
            .sensitive_extensions()
            .iter()
            .map(|ext| format!("*.{ext}")),
    );
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
    let input_tx = runtime.handle().input_tx.clone();
    Ok((runtime, input_tx, input_rx, shutdown_token))
}

#[cfg(test)]
mod projection_recovery_tests;

#[cfg(test)]
mod parser_resume_tests;

#[cfg(test)]
mod recovery_input_tests;

#[cfg(test)]
mod recovery_scope_tests;

#[cfg(test)]
mod approval_recovery_tests;

#[cfg(test)]
mod runtime_supervision_tests;
