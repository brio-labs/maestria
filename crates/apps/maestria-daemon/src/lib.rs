use std::{fs, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceService};
use maestria_domain::{DomainInput, KernelState, replay_events};
use maestria_governance::{AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier, Scope};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_parsers::ParserRegistry;
use maestria_ports::{EventFilter, InMemoryHarnessAdapter};
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
use maestria_web_evidence::UreqWebFetcher;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;

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
    let blob_store = Arc::new(
        FsBlobStore::open(&layout.blobs_dir)
            .with_context(|| format!("open blob store {}", layout.blobs_dir.display()))?,
    );
    let search_index = Arc::new(
        TantivyFullTextIndex::open(&layout.full_text_index_dir).with_context(|| {
            format!(
                "open full-text index {}",
                layout.full_text_index_dir.display()
            )
        })?,
    );
    let parser = Arc::new(ParserRegistry::with_defaults());
    let sqlite_store = Arc::new(
        SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?,
    );
    let event_log = sqlite_store.clone();
    let artifact_repo = sqlite_store.clone();
    let harness = Arc::new(InMemoryHarnessAdapter::default());
    let chunk_repo = sqlite_store.clone();
    let card_repo = sqlite_store.clone();
    let evidence_repo = sqlite_store.clone();
    let vector_index = Arc::new(
        SqliteVectorIndex::open(&layout.vector_index_dir.join("projection.db"))
            .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?,
    );
    let graph_index = Arc::new(
        SqliteGraphIndex::open(&layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    let web_fetcher = Arc::new(UreqWebFetcher::new());

    let adapters = Adapters {
        event_log,
        blob_store,
        search_index,
        parser,
        harness,
        artifact_repo,
        chunk_repo,
        card_repo,
        evidence_repo,
        vector_index,
        graph_index,
        web_fetcher,
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let manifest_contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceService::parse_manifest(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let scope = Scope::new(
        manifest.read_roots,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
    );
    let config = RuntimeConfig {
        profile,
        scope,
        ..Default::default()
    };

    let shutdown_token = CancellationToken::new();
    let (runtime, input_rx) = MaestriaRuntime::new(config, state, adapters, governance);
    let input_tx = runtime.handle().input_tx.clone();
    Ok((runtime, input_tx, input_rx, shutdown_token))
}

pub async fn run_instance(instance_dir: PathBuf) -> Result<()> {
    let layout = prepare_instance(instance_dir).with_context(|| "prepare instance layout")?;
    let state = load_kernel_state(&layout).with_context(|| "load persisted kernel state")?;
    let (runtime, _input_tx, input_rx, shutdown_token) = build_runtime(&layout, state, AutonomyProfile::ReadOnly)?;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let root = layout.root.clone();
    info!(root = %root.display(), "runtime started");

    tokio::signal::ctrl_c().await?;
    info!(root = %root.display(), "shutdown requested");
    shutdown_token.cancel();

    runtime_task
        .await
        .with_context(|| "runtime loop join failed")?;

    Ok(())
}
