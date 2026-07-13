use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceManifest, InstanceService};
use maestria_domain::{ArtifactId, DomainInput, KernelState, StartFullTextIndex, replay_events};
use maestria_governance::{
    AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier, PrivacyExclusions, Scope,
};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_harness::LocalShellHarnessAdapter;
use maestria_parsers::ParserRegistry;
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EventFilter, EvidenceRepository,
};
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
/// `StartFullTextIndex` inputs so the runtime can resume indexing after
/// restart without re-parsing source bytes or re-playing `ParserCompleted`.
fn pending_start_full_text(state: &KernelState) -> Vec<DomainInput> {
    let mut artifacts: BTreeSet<ArtifactId> = BTreeSet::new();
    for chunk_id in &state.pending_full_text {
        if let Some(chunk) = state.chunks.get(chunk_id) {
            // Skip artifacts that have a pending parser — the resumed
            // parser flow owns completion, evidence, and index ordering
            // and emits its own StartFullTextIndex afterward.  Issuing a
            // separate StartFullTextIndex here could make chunks terminal
            // before resumed evidence is recorded.
            if state.pending_parsers.contains_key(&chunk.artifact_id) {
                continue;
            }
            artifacts.insert(chunk.artifact_id);
        }
    }
    artifacts
        .into_iter()
        .map(|artifact_id| DomainInput::StartFullTextIndex(StartFullTextIndex { artifact_id }))
        .collect()
}
/// Typed container for pending recovery inputs computed from replayed kernel state.
///
/// `resume_parsers` carries `ResumeParser` inputs for artifacts whose parsing was
/// interrupted (ParserStarted without ParserCompleted). `start_full_text` carries
/// `StartFullTextIndex` inputs for artifacts whose chunks are pending full-text
/// indexing but that are _not_ covered by a pending parser — the parser flow owns
/// its own index dispatch and emits `StartFullTextIndex` after completion.
///
/// Callers MUST enqueue `resume_parsers` before `start_full_text` to preserve
/// bounded-channel ordering: parser completion creates chunks, and full-text
/// indexing needs those chunks to exist.
#[derive(Debug, Clone)]
pub struct RecoveryInputs {
    pub resume_parsers: Vec<DomainInput>,
    pub start_full_text: Vec<DomainInput>,
}

/// Compute deterministic recovery inputs from replayed kernel state.
///
/// Returns `RecoveryInputs` with `ResumeParser` inputs first (derived from
/// `pending_parsers`) and `StartFullTextIndex` inputs for non-parser-pending
/// artifacts (derived from `pending_full_text`).
pub fn recovery_inputs(state: &KernelState) -> RecoveryInputs {
    let resume_parsers = pending_resume_parsers(state);
    let start_full_text = pending_start_full_text(state);
    RecoveryInputs {
        resume_parsers,
        start_full_text,
    }
}

mod parser_resume;
use parser_resume::pending_resume_parsers;
pub use parser_resume::verify_pending_blobs;

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

/// Reconcile projection repositories from replayed domain truth.
///
/// After `load_kernel_state` replays the event log into a `KernelState`,
/// this helper idempotently upserts every artifact, chunk, and card,
/// and unconditionally replaces every evidence row from the replayed
/// state into the SQLite projection tables.  Evidence uses `replace`
/// so a valid replayed row overwrites a stale, malformed, or partial
/// row from a prior crash without tripping a `Conflict` error.
///
/// Projection repair never emits domain events and never changes event
/// truth.  Startup recovery can then search/open evidence even if the
/// previous process crashed after event append but before a projection
/// write.
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
    {
        let store = SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        reconcile_projections(&state, &store)
            .with_context(|| "reconcile projection repositories")?;
    }

    // Compute recovery inputs before state is moved into build_runtime.
    let recovery = recovery_inputs(&state);

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
