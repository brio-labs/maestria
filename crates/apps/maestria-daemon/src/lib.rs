use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceManifest, InstanceService};
#[cfg(test)]
use maestria_domain::ArtifactId;
use maestria_domain::{DomainInput, KernelState, replay_events};
use maestria_governance::{
    AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate,
    PrivacyExclusions, Scope,
};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_harness::LocalShellHarnessAdapter;
use maestria_parsers::ParserRegistry;
use maestria_ports::EventFilter;
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
use maestria_web_evidence::UreqWebFetcher;
use tokio::{
    sync::mpsc,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::info;
pub struct InstanceWriteLock {
    path: PathBuf,
    token: String,
}

impl Drop for InstanceWriteLock {
    fn drop(&mut self) {
        let owned =
            fs::read_to_string(&self.path).is_ok_and(|contents| contents.trim() == self.token);
        if owned && let Err(error) = fs::remove_file(&self.path) {
            tracing::warn!(path = %self.path.display(), %error, "failed to release instance write lock");
        }
    }
}

pub fn try_acquire_instance_write_lock(
    layout: &InstanceLayout,
) -> Result<Option<InstanceWriteLock>> {
    let path = layout.system_dir.join("instance-write.lock");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let token = format!("{}:{nonce}", std::process::id());
            if let Err(error) = writeln!(file, "{token}") {
                let _ = fs::remove_file(&path);
                return Err(error)
                    .with_context(|| format!("write instance lock {}", path.display()));
            }
            Ok(Some(InstanceWriteLock { path, token }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !lock_owner_is_dead(&path) {
                return Ok(None);
            }
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let quarantine = path.with_extension(format!("stale.{}.{}", std::process::id(), nonce));
            match fs::hard_link(&path, &quarantine) {
                Ok(()) => match fs::remove_file(&path) {
                    Ok(()) => {
                        fs::remove_file(&quarantine).with_context(|| {
                            format!("remove stale instance lock {}", quarantine.display())
                        })?;
                        try_acquire_instance_write_lock(layout)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let _ = fs::remove_file(&quarantine);
                        try_acquire_instance_write_lock(layout)
                    }
                    Err(error) => Err(error)
                        .with_context(|| format!("remove stale instance lock {}", path.display())),
                },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
                Err(error) => Err(error)
                    .with_context(|| format!("quarantine instance lock {}", path.display())),
            }
        }
        Err(error) => {
            Err(error).with_context(|| format!("create instance lock {}", path.display()))
        }
    }
}

pub async fn acquire_instance_write_lock(layout: &InstanceLayout) -> Result<InstanceWriteLock> {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Some(lock) = try_acquire_instance_write_lock(layout)? {
                return Ok(lock);
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("timed out waiting for instance write lock"))?
}
fn lock_owner_is_dead(path: &PathBuf) -> bool {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => {
            return fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                .is_ok_and(|age| age > Duration::from_secs(30));
        }
    };
    let pid_text = contents
        .trim()
        .split_once(':')
        .map_or(contents.trim(), |(pid, _)| pid);
    let Ok(pid) = pid_text.parse::<u32>() else {
        return fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .is_ok_and(|age| age > Duration::from_secs(30));
    };
    #[cfg(target_os = "linux")]
    {
        !PathBuf::from(format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

mod approval_recovery;
mod projection_recovery;
pub use approval_recovery::{reconcile_approval_repo, reconcile_pending_approvals};
pub use projection_recovery::reconcile_projections;
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
    let manifest_contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceService::parse_manifest(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let embedding_model = manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| config.model.clone());
    let embedding_provider = match manifest.embeddings.as_ref().filter(|config| config.enabled) {
        Some(config) => Some(
            Arc::new(maestria_embedding_openai::LocalHttpEmbeddingProvider::new(
                &config.endpoint,
                &config.model,
                Some(config.dimensions),
            )?) as Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>,
        ),
        None => None,
    };
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
    let harness = Arc::new(LocalShellHarnessAdapter);
    let chunk_repo = sqlite_store.clone();
    let card_repo = sqlite_store.clone();
    let evidence_repo = sqlite_store.clone();
    let vector_index = Arc::new(
        SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
            .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?,
    );
    let graph_index = Arc::new(
        SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    let web_fetcher = Arc::new(UreqWebFetcher::new());

    let id_allocator = sqlite_store.clone();
    let approval_repo = sqlite_store.clone();

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
        embedding_provider,
        vector_index,
        graph_index,
        web_fetcher,
        id_allocator,
        effect_journal: sqlite_store.clone(),
        approval_repo,
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
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

pub async fn run_instance(instance_dir: PathBuf) -> Result<()> {
    let layout = prepare_instance(instance_dir).with_context(|| "prepare instance layout")?;
    let _instance_lock = acquire_instance_write_lock(&layout).await?;
    let state = load_kernel_state(&layout).with_context(|| "load persisted kernel state")?;

    // Repair projection repositories before runtime start so that
    // artifact, chunk, card, and evidence lookups succeed even if the
    // previous process crashed after event append but before a
    // projection write.
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    {
        reconcile_projections(&state, &store)
            .with_context(|| "reconcile projection repositories")?;
        reconcile_approval_repo(&state, &store).with_context(|| "reconcile approval repository")?;
        reconcile_pending_approvals(&state, &store, &store)
            .with_context(|| "reconcile pending approvals")?;
    }
    // Compute recovery diagnostics and pause in-flight harness effects before state is moved.
    let diagnostics = supervise_recovery(&state, &store)?;
    let recovery = diagnostics.inputs;
    tracing::info!(
        paused_effects = diagnostics.paused_effects.len(),
        "recovery diagnostics computed"
    );

    // Validate recovery scope from the instance manifest before touching
    // blobs or building the runtime.  Out-of-scope and excluded pending
    // parsers fail fast with a descriptive error.
    validate_recovery_scope(&layout, &recovery)
        .with_context(|| "validate recovery scope against instance manifest")?;

    // Verify pending parser blobs exist before building the runtime so
    // missing-blob errors surface early instead of silently dropping work.
    verify_pending_blobs(&layout, &recovery.resume_parsers)
        .with_context(|| "verify pending parser blobs for resume")?;

    let (runtime, input_tx, input_rx, shutdown_token) =
        build_runtime(&layout, state, AutonomyProfile::ReadOnly)?;

    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
        // Submit pending ResumeParser inputs first so that parsing (which
        // creates chunks) completes before full-text indexing begins.
        for input in recovery.resume_parsers {
            input_tx
                .send(input)
                .await
                .map_err(|e| anyhow!("failed to queue resume parser: {e}"))?;
        }

        // Submit pending StartFullTextIndex inputs after the runtime task has
        // started consuming from `input_rx` to avoid bounded-channel deadlock.
        for input in recovery.start_full_text {
            input_tx
                .send(input)
                .await
                .map_err(|e| anyhow!("failed to queue restart full-text index: {e}"))?;
        }

        for input in recovery.run_validations {
            input_tx
                .send(input)
                .await
                .map_err(|e| anyhow!("failed to queue request task validation: {e}"))?;
        }

        let root = layout.root.clone();
        info!(root = %root.display(), "runtime started");
        tokio::signal::ctrl_c()
            .await
            .with_context(|| "wait for shutdown signal")
    }
    .await;

    info!(root = %layout.root.display(), "shutdown requested");
    shutdown_token.cancel();
    let join_result = runtime_task.await;
    result?;
    join_result.with_context(|| "runtime loop join failed")?;

    Ok(())
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
