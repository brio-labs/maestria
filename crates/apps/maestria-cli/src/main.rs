use anyhow::{Context, Result, anyhow};
use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{
    CorePorts, CoreServices, InstanceLayout, InstanceManifest, InstanceService,
    OpenChunkEvidenceInput, OpenEvidenceInput, SearchInput, artifact_id_for, content_hash,
};
use maestria_domain::{
    ArtifactDetected, ArtifactId, ChunkId, DomainInput, EvidenceId, EvidenceKind, IndexStatus,
    KernelState, MemoryCandidate, OpenTaskInput, Task, TaskId, TaskPriority,
};
use maestria_governance::{AutonomyProfile, PrivacyExclusions, Scope};
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::time::{sleep, timeout};

#[derive(ClapParser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a local Maestria instance layout
    Init {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        /// Approved root paths that may be indexed by this instance
        #[arg(long = "read-root")]
        read_roots: Vec<PathBuf>,
    },
    /// Index one local file, or files under a directory with --recursive
    Index {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        path: PathBuf,
        #[arg(short, long)]
        recursive: bool,
    },
    /// Search indexed local chunks
    Search {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Resolve typed source evidence without launching external programs
    #[command(alias = "evidence")]
    OpenEvidence {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long, conflicts_with = "chunk_id")]
        evidence_id: Option<u64>,
        #[arg(long, conflicts_with = "evidence_id")]
        chunk_id: Option<u64>,
    },
    /// Print local instance health facts
    Status {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Check local storage, index, blob, and parser wiring
    Doctor {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Start the daemon
    Start {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Task workflow commands
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    /// Memory projection commands
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
}

#[derive(Subcommand)]
enum TaskCommands {
    /// Create a new task in persisted task state
    Start {
        /// Optional task title when provided from command line args
        title: String,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(short, long, default_value = "normal")]
        priority: CliTaskPriority,
        #[arg(short, long)]
        artifact_id: Option<u64>,
    },
    /// Show all tasks or a single task
    Show {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        task_id: Option<u64>,
    },
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// List persisted memory candidates
    Candidates {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliTaskPriority {
    Low,
    Normal,
    High,
}

impl std::fmt::Display for CliTaskPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            CliTaskPriority::Low => "low",
            CliTaskPriority::Normal => "normal",
            CliTaskPriority::High => "high",
        };
        write!(f, "{label}")
    }
}

impl From<CliTaskPriority> for TaskPriority {
    fn from(value: CliTaskPriority) -> Self {
        match value {
            CliTaskPriority::Low => TaskPriority::Low,
            CliTaskPriority::Normal => TaskPriority::Normal,
            CliTaskPriority::High => TaskPriority::High,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            instance_dir,
            read_roots,
        } => init_instance(instance_dir, read_roots)?,
        Commands::Index {
            instance_dir,
            path,
            recursive,
        } => index_path(instance_dir, path, recursive).await?,
        Commands::Search {
            instance_dir,
            query,
            limit,
        } => search(instance_dir, query, limit)?,
        Commands::OpenEvidence {
            instance_dir,
            evidence_id,
            chunk_id,
        } => open_evidence(instance_dir, evidence_id, chunk_id)?,
        Commands::Status { instance_dir } => status(instance_dir)?,
        Commands::Doctor { instance_dir } => doctor(instance_dir)?,
        Commands::Start { instance_dir } => maestria_daemon::run_instance(instance_dir).await?,
        Commands::Task { command } => match command {
            TaskCommands::Start {
                title,
                instance_dir,
                priority,
                artifact_id,
            } => {
                task_start(instance_dir, title, priority, artifact_id).await?;
            }
            TaskCommands::Show {
                instance_dir,
                task_id,
            } => {
                task_show(instance_dir, task_id)?;
            }
        },
        Commands::Memory { command } => match command {
            MemoryCommands::Candidates {
                instance_dir,
                limit,
            } => memory_candidates(instance_dir, limit)?,
        },
    }

    Ok(())
}

fn init_instance(instance_dir: PathBuf, read_roots: Vec<PathBuf>) -> Result<()> {
    let read_roots = if read_roots.is_empty() {
        vec![instance_dir.clone()]
    } else {
        read_roots
    };
    let plan = InstanceService::init_instance_with_roots(instance_dir, read_roots)?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    println!("initialized {}", plan.layout.root.display());
    println!("manifest {}", plan.manifest_path.display());
    Ok(())
}

async fn index_path(instance_dir: PathBuf, path: PathBuf, recursive: bool) -> Result<()> {
    let layout = ensure_instance(instance_dir)?;
    let manifest = load_manifest(&layout)?;
    let scope = Scope::new(
        manifest.read_roots.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
    );
    let privacy = PrivacyExclusions::default();

    let files = collect_index_files(&path, recursive)?;
    if files.is_empty() {
        return Err(anyhow!(
            "no files selected for indexing at {}",
            path.display()
        ));
    }

    // Load persistent kernel state to check which artifacts are already indexed.
    let initial_state = maestria_daemon::load_kernel_state(&layout)
        .with_context(|| "load kernel state for indexing")?;

    // Repair projection repositories before runtime start so that
    // artifact, chunk, card, and evidence lookups succeed even if the
    // previous process crashed after event append but before a
    // projection write.
    {
        let store = SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        maestria_daemon::reconcile_projections(&initial_state, &store)
            .with_context(|| "reconcile projection repositories")?;
    }

    let preexisting_state = initial_state.clone();
    // Compute recovery inputs before state is moved into build_runtime.
    // Missing pending blobs surface as an error rather than silently dropping work.
    let recovery = maestria_daemon::recovery_inputs(&initial_state);

    // Validate recovery scope from the instance manifest before touching
    // blobs or building the runtime.  Out-of-scope and excluded pending
    // parsers fail fast with a descriptive error.
    maestria_daemon::validate_recovery_scope(&layout, &recovery)
        .with_context(|| "validate recovery scope against instance manifest")?;

    maestria_daemon::verify_pending_blobs(&layout, &recovery.resume_parsers)
        .with_context(|| "verify pending parser blobs for resume")?;

    // Build a one-shot runtime with a non-critical profile that allows
    // PersistEvent / StoreBlob / ParseArtifact effects.
    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, initial_state, AutonomyProfile::TrustedWorkspace)?;

    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));
    // Collect recovery artifact IDs before consuming the recovery struct so
    // we can drain pending recovery work before shutdown.  Each recovery
    // artifact must reach terminal IndexStatus::Indexed before the runtime
    // is cancelled, otherwise unrelated pending work is silently aborted.
    let recovery_artifact_ids: Vec<ArtifactId> = {
        let resume_ids = recovery.resume_parsers.iter().filter_map(|input| {
            if let DomainInput::ResumeParser(r) = input {
                Some(r.artifact_id)
            } else {
                None
            }
        });
        let ft_ids = recovery.start_full_text.iter().filter_map(|input| {
            if let DomainInput::StartFullTextIndex(s) = input {
                Some(s.artifact_id)
            } else {
                None
            }
        });
        resume_ids.chain(ft_ids).collect()
    };

    // Queue pending ResumeParser inputs first so parsing completes before
    // full-text indexing begins.
    for input in recovery.resume_parsers {
        input_tx
            .send(input)
            .await
            .context("failed to queue resume parser")?;
    }

    // Queue pending StartFullTextIndex inputs after resume parsers.
    for input in recovery.start_full_text {
        input_tx
            .send(input)
            .await
            .context("failed to queue restart full-text index")?;
    }

    let index_timeout = Duration::from_secs(30);

    for file in files {
        // Preserve scope, privacy, and manifest checks before reading.
        if scope.check_read_containment(&file).is_err()
            || privacy.is_excluded(&file)
            || !manifest.allows_source(&file)
        {
            shutdown_token.cancel();
            let _ = runtime_task.await;
            return Err(anyhow!(
                "index path is outside the instance read scope or excluded by policy: {}",
                file.display()
            ));
        }

        let bytes = fs::read(&file)?;
        let artifact_id = artifact_id_for(&file, &bytes);
        let hash = content_hash(&bytes);
        // Check whether this exact artifact was already indexed before this session.
        if let Some(artifact) = preexisting_state.artifacts.get(&artifact_id)
            && artifact.content_hash.as_deref() == Some(&hash)
            && artifact.index_status == IndexStatus::Indexed
        {
            println!("unchanged artifact={} path={}", artifact.id, file.display());
            continue;
        }

        let title = match file.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => "unknown".to_string(),
        };

        input_tx
            .send(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title,
                source_path: file.display().to_string(),
                source_bytes: bytes,
                content_hash: hash,
            }))
            .await
            .context("failed to submit artifact to runtime")?;

        // Wait for the artifact to reach terminal persisted state.
        let result = timeout(index_timeout, async {
            loop {
                match maestria_daemon::load_kernel_state(&layout)
                    .with_context(|| "reload kernel state for indexing wait")
                {
                    Ok(state) => {
                        if let Some(artifact) = state.artifacts.get(&artifact_id)
                            && artifact.index_status == IndexStatus::Indexed
                        {
                            println!("indexed artifact={} path={}", artifact.id, file.display());
                            return Ok::<_, anyhow::Error>(());
                        }
                        sleep(Duration::from_millis(100)).await;
                    }
                    Err(error) if is_db_locked(&error) => {
                        sleep(Duration::from_millis(25)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                shutdown_token.cancel();
                let _ = runtime_task.await;
                return Err(error);
            }
            Err(_elapsed) => {
                shutdown_token.cancel();
                let _ = runtime_task.await;
                return Err(anyhow!(
                    "timeout waiting for artifact indexing: {}",
                    file.display()
                ));
            }
        }
    }

    // Drain every recovery artifact to terminal Indexed state before
    // shutdown.  Recovery inputs (resumed parsers and restarted full-text
    // indexing) run concurrently with fresh file processing; this wait
    // ensures pending recovery work is not silently aborted when the
    // runtime shuts down.
    if !recovery_artifact_ids.is_empty() {
        let recovery_timeout = Duration::from_secs(60);
        let result = timeout(recovery_timeout, async {
            loop {
                match maestria_daemon::load_kernel_state(&layout)
                    .with_context(|| "reload kernel state for recovery drain")
                {
                    Ok(state) => {
                        if recovery_artifact_ids.iter().all(|id| {
                            state
                                .artifacts
                                .get(id)
                                .is_some_and(|a| a.index_status == IndexStatus::Indexed)
                        }) {
                            return Ok::<_, anyhow::Error>(());
                        }
                        sleep(Duration::from_millis(100)).await;
                    }
                    Err(error) if is_db_locked(&error) => {
                        sleep(Duration::from_millis(25)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                shutdown_token.cancel();
                let _ = runtime_task.await;
                return Err(error);
            }
            Err(_elapsed) => {
                shutdown_token.cancel();
                let _ = runtime_task.await;
                return Err(anyhow!("timeout waiting for recovery artifact indexing"));
            }
        }
    }

    // Clean shutdown.
    shutdown_token.cancel();
    let _ = runtime_task.await;

    Ok(())
}

fn search(instance_dir: PathBuf, query: String, limit: usize) -> Result<()> {
    let layout = validated_instance(instance_dir)?;
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite_store,
        chunks: &sqlite_store,
        cards: &sqlite_store,
        evidence: &sqlite_store,
        events: &sqlite_store,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
    });

    let output = core.search(SearchInput { query, limit })?;
    for hit in output.hits {
        let source = source_label(&hit.evidence);
        println!(
            "score={} artifact={} chunk={} {} snippet={}",
            hit.score,
            hit.artifact.id,
            hit.chunk.id,
            source,
            hit.chunk
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    Ok(())
}

fn open_evidence(
    instance_dir: PathBuf,
    evidence_id: Option<u64>,
    chunk_id: Option<u64>,
) -> Result<()> {
    let layout = validated_instance(instance_dir)?;
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite_store,
        chunks: &sqlite_store,
        cards: &sqlite_store,
        evidence: &sqlite_store,
        events: &sqlite_store,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
    });

    let output = if let Some(id) = evidence_id {
        core.open_evidence(OpenEvidenceInput {
            evidence_id: EvidenceId::new(id),
        })?
    } else if let Some(id) = chunk_id {
        core.open_chunk_evidence(OpenChunkEvidenceInput {
            chunk_id: ChunkId::new(id),
        })?
    } else {
        return Err(anyhow!("provide --evidence-id or --chunk-id"));
    };

    println!(
        "artifact={} title={}",
        output.artifact.id, output.artifact.title
    );
    println!(
        "evidence={} {}",
        output.evidence.id,
        source_label(&output.evidence)
    );
    println!("excerpt={}", output.evidence.excerpt);
    Ok(())
}

fn status(instance_dir: PathBuf) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let state = maestria_daemon::load_kernel_state(&layout).with_context(|| "load kernel state")?;
    println!("instance {}", layout.root.display());
    println!("database {}", layout.database_path.display());
    println!("full_text_index {}", layout.full_text_index_dir.display());
    println!("events {}", state.event_log.len());
    Ok(())
}

fn doctor(instance_dir: PathBuf) -> Result<()> {
    let layout = ensure_instance(instance_dir)?;
    let _sqlite_store = SqliteStore::open(&layout.database_path)?;
    let _blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let _search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    println!("ok instance {}", layout.root.display());
    println!("ok database {}", layout.database_path.display());
    println!("ok blobs {}", layout.blobs_dir.display());
    println!(
        "ok full_text_index {}",
        layout.full_text_index_dir.display()
    );
    println!("ok parsers {}", parser.parser_count());
    Ok(())
}

async fn task_start(
    instance_dir: PathBuf,
    title: String,
    priority: CliTaskPriority,
    artifact_id: Option<u64>,
) -> Result<()> {
    let layout = ensure_instance(instance_dir)?;
    let state = load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before task start",
    )
    .await?;
    let task_id = next_task_id(&state);
    create_task_workspace_directories(&layout, task_id)?;

    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, state, AutonomyProfile::TrustedWorkspace)
            .with_context(|| "build runtime")?;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let input = DomainInput::OpenTask(OpenTaskInput {
        task_id,
        title,
        priority: priority.into(),
        artifact_id: artifact_id.map(ArtifactId::new),
    });
    input_tx
        .send(input)
        .await
        .map_err(|error| anyhow!("failed to queue task open input: {error}"))?;

    let state = wait_for_task_in_state(&layout, task_id, Duration::from_secs(2)).await?;
    shutdown_token.cancel();
    runtime_task
        .await
        .with_context(|| "runtime loop join failed")?;

    let task = state
        .tasks
        .get(&task_id)
        .cloned()
        .ok_or_else(|| anyhow!("task {} was not persisted", task_id))?;

    println!(
        "task={} title={} status={:?} priority={:?}",
        task.id, task.title, task.status, task.priority
    );

    Ok(())
}

const TASK_WORKSPACE_SUBDIRECTORIES: [&str; 5] =
    ["context", "evidence", "drafts", "validation", "artifacts"];

fn task_workspace_directory(layout: &InstanceLayout, task_id: TaskId) -> PathBuf {
    layout
        .active_tasks_dir
        .join(format!("task_{}", task_id.value()))
}

fn create_task_workspace_directories(layout: &InstanceLayout, task_id: TaskId) -> Result<()> {
    let task_directory = task_workspace_directory(layout, task_id);
    fs::create_dir_all(&task_directory).with_context(|| {
        format!(
            "failed to create task workspace {} for task {}",
            task_directory.display(),
            task_id
        )
    })?;

    for subdirectory in TASK_WORKSPACE_SUBDIRECTORIES {
        let path = task_directory.join(subdirectory);
        fs::create_dir_all(&path).with_context(|| {
            format!(
                "failed to create task {task_id} {} directory {}",
                subdirectory,
                path.display()
            )
        })?;
    }

    Ok(())
}

async fn wait_for_task_in_state(
    layout: &InstanceLayout,
    task_id: TaskId,
    timeout_budget: Duration,
) -> Result<KernelState> {
    use std::sync::Mutex;

    let last_error = Arc::new(Mutex::new(None::<String>));
    let last_error_for_wait = last_error.clone();
    timeout(timeout_budget, async move {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| "load kernel state while waiting for task persistence")
            {
                Ok(state) => {
                    if state.tasks.contains_key(&task_id) {
                        return Ok(state);
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if is_db_locked(&error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    return Err(error);
                }
            }
        }
    })
    .await
    .map_err(|_| {
        let detail = last_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .map_or_else(String::new, |error| format!(" {error}"));
        anyhow!("timed out while waiting for task {task_id} persistence{detail}")
    })?
}

async fn load_kernel_state_with_retry(
    layout: &InstanceLayout,
    timeout_budget: Duration,
    context: &'static str,
) -> Result<KernelState> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout).with_context(|| context) {
                Ok(state) => return Ok(state),
                Err(error) if is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while {context}"))?
}

fn is_db_locked(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_lowercase();
    message.contains("database is locked")
        || message.contains("database is busy")
        || message.contains("locked")
}

fn task_show(instance_dir: PathBuf, task_id: Option<u64>) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let state = maestria_daemon::load_kernel_state(&layout).with_context(|| "load kernel state")?;

    if let Some(requested) = task_id {
        let requested = TaskId::new(requested);
        let task = state
            .tasks
            .get(&requested)
            .ok_or_else(|| anyhow!("task {} not found", requested))?;
        print_task(task);
        return Ok(());
    }

    if state.tasks.is_empty() {
        println!("no tasks");
        return Ok(());
    }

    for task in state.tasks.values() {
        print_task(task);
    }

    Ok(())
}

fn memory_candidates(instance_dir: PathBuf, limit: usize) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let state = maestria_daemon::load_kernel_state(&layout).with_context(|| "load kernel state")?;

    if state.memory_candidates.is_empty() {
        println!("no memory candidates");
        return Ok(());
    }

    for candidate in state.memory_candidates.values().take(limit) {
        print_memory_candidate(candidate);
    }

    Ok(())
}

fn print_task(task: &Task) {
    print!(
        "task={} status={:?} priority={:?} title='{}'",
        task.id, task.status, task.priority, task.title
    );

    if let Some(report_id) = task.validation_report_id {
        print!(" validation_report={report_id}");
    }

    if !task.artifact_ids.is_empty() {
        print!(" artifacts={:?}", task.artifact_ids);
    }

    if !task.evidence_ids.is_empty() {
        print!(" evidence={:?}", task.evidence_ids);
    }

    println!();
}

fn print_memory_candidate(candidate: &MemoryCandidate) {
    println!(
        "candidate={} claim={} confidence={} evidence={} ids={:?}",
        candidate.id,
        candidate.claim_id,
        candidate.confidence_milli,
        candidate.evidence_ids.len(),
        candidate.evidence_ids
    );
}

fn next_task_id(state: &maestria_domain::KernelState) -> TaskId {
    state
        .tasks
        .iter()
        .next_back()
        .map_or(TaskId::new(1), |(id, _)| TaskId::new(id.value() + 1))
}

fn ensure_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    maestria_daemon::prepare_instance(instance_dir)
}

fn validated_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let layout = InstanceLayout::for_root(instance_dir);
    if !layout.manifest_path.exists() {
        return Err(anyhow!(
            "instance manifest is missing at {}; run init first",
            layout.manifest_path.display()
        ));
    }
    load_manifest(&layout)?;
    Ok(layout)
}

fn load_manifest(layout: &InstanceLayout) -> Result<InstanceManifest> {
    let contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    InstanceService::parse_manifest(&contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))
}

fn collect_index_files(path: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    if is_excluded_index_path(path) {
        return Err(anyhow!(
            "index path is excluded by privacy policy: {}",
            path.display()
        ));
    }
    if is_symlink(path)? {
        return Err(anyhow!(
            "index path is a symlink and is not indexed: {}",
            path.display()
        ));
    }
    if path.is_file() {
        if !is_supported_index_path(path) {
            return Err(anyhow!("unsupported index file type: {}", path.display()));
        }
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(anyhow!("index path does not exist: {}", path.display()));
    }
    if !recursive {
        return Err(anyhow!(
            "{} is a directory; pass --recursive to index contained files",
            path.display()
        ));
    }

    let mut files = Vec::new();
    collect_files_recursive(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_recursive(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink()
            || is_excluded_index_path(&path)
            || (path.is_file() && !is_supported_index_path(&path))
        {
            continue;
        }
        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn is_excluded_index_path(path: &Path) -> bool {
    let default_exclusions = PrivacyExclusions::default();
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".ssh" | ".gnupg" | "node_modules" | "target" | "dist" | "build"
        ) || name.starts_with(".env.")
    }) || default_exclusions.is_excluded(path)
}

fn is_symlink(path: &Path) -> Result<bool> {
    Ok(fs::symlink_metadata(path)?.file_type().is_symlink())
}

fn is_supported_index_path(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt" | "text" | "rs" | "pdf")
    )
}

fn source_label(evidence: &maestria_domain::Evidence) -> String {
    match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
            ..
        } => format!(
            "source=file path={} lines={}-{} hash={}",
            path, range.start, range.end, content_hash
        ),
        EvidenceKind::PdfSpan {
            blob,
            page_start,
            page_end,
        } => format!("source=pdf blob={} pages={}-{}", blob, page_start, page_end),
        EvidenceKind::WebSnapshot { url, snapshot, .. } => {
            format!("source=web url={} snapshot={}", url, snapshot)
        }
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            blob,
        } => format!(
            "source=command run={} stream={:?} blob={}",
            harness_run, stream, blob
        ),
        EvidenceKind::TestResult {
            harness_run,
            status,
            log,
        } => format!(
            "source=test run={} status={:?} log={}",
            harness_run, status, log
        ),
        EvidenceKind::Diff {
            harness_run,
            patch_blob,
        } => format!("source=diff run={} patch={}", harness_run, patch_blob),
        EvidenceKind::Validation { report_id } => {
            format!("source=validation report={}", report_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn create() -> Self {
            let base = std::env::temp_dir();
            for _ in 0..1000 {
                let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!(
                    "maestria-cli-index-test-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create test directory {}: {error}", path.display()),
                }
            }
            panic!("create unique test directory under {}", base.display());
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, contents).expect("write test file");
    }

    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    fn symlink_unavailable(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
        )
    }

    fn relative_files(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
        files
            .iter()
            .map(|path| {
                path.strip_prefix(root)
                    .expect("collected file stays under root")
                    .to_path_buf()
            })
            .collect()
    }

    #[test]
    fn index_exclusion_policy_covers_sensitive_and_build_paths() {
        for path in [
            "workspace/.env",
            "workspace/.env.local",
            "workspace/cert.pem",
            "workspace/deploy.key",
            "workspace/secrets/token.md",
            "workspace/.ssh/config",
            "workspace/.gnupg/pubring.kbx",
            "workspace/node_modules/package/index.js",
            "workspace/target/debug/app",
            "workspace/dist/bundle.js",
            "workspace/build/output.o",
        ] {
            assert!(
                is_excluded_index_path(Path::new(path)),
                "expected {path} to be excluded from indexing"
            );
        }

        for path in [
            "workspace/notes/readme.md",
            "workspace/src/building.md",
            "workspace/src/targeted.md",
        ] {
            assert!(
                !is_excluded_index_path(Path::new(path)),
                "expected {path} to be indexable"
            );
        }
    }

    #[test]
    fn collecting_single_env_file_is_rejected_by_privacy_policy() {
        let directory = TestDirectory::create();
        let env_file = directory.path().join(".env");
        write_file(&env_file, "TOKEN=secret");

        let error = collect_index_files(&env_file, false)
            .expect_err("single .env files must not be accepted for indexing");

        assert!(
            error.to_string().contains("privacy policy"),
            "unexpected error for excluded .env file: {error}"
        );
    }

    #[test]
    fn collecting_single_unsupported_file_is_rejected() {
        let directory = TestDirectory::create();
        let unsupported_file = directory.path().join("notes.sqlite");
        write_file(&unsupported_file, "not text evidence");

        let error = collect_index_files(&unsupported_file, false)
            .expect_err("single unsupported files must not be accepted for indexing");

        assert!(
            error.to_string().contains("unsupported index file type"),
            "unexpected error for unsupported file: {error}"
        );
    }

    #[test]
    fn pdf_is_supported_index_path() {
        assert!(is_supported_index_path(Path::new("paper.pdf")));
        assert!(is_supported_index_path(Path::new("paper.PDF")));
        assert!(is_supported_index_path(Path::new("docs/report.Pdf")));
    }

    #[test]
    fn collecting_single_pdf_is_accepted() {
        let directory = TestDirectory::create();
        let pdf_file = directory.path().join("paper.pdf");
        write_file(&pdf_file, "minimal pdf bytes");

        let files =
            collect_index_files(&pdf_file, false).expect("single PDF files must be accepted");

        assert_eq!(files, vec![pdf_file]);
    }

    #[test]
    fn recursive_collection_includes_pdf_files() {
        let directory = TestDirectory::create();
        write_file(&directory.path().join("note.md"), "# Normal note");
        write_file(
            &directory.path().join("docs/report.pdf"),
            "minimal pdf bytes",
        );
        write_file(
            &directory.path().join("docs/cache.sqlite"),
            "opaque database",
        );

        let files =
            collect_index_files(directory.path(), true).expect("recursive collection succeeds");

        assert_eq!(
            relative_files(directory.path(), &files),
            vec![PathBuf::from("docs/report.pdf"), PathBuf::from("note.md"),]
        );
    }

    #[test]
    fn collecting_single_symlink_is_rejected_and_recursive_collection_skips_it() {
        let directory = TestDirectory::create();
        let sensitive_target = directory.path().join(".env");
        let benign_link = directory.path().join("public.md");
        let supported_note = directory.path().join("note.md");
        write_file(&sensitive_target, "TOKEN=secret");
        write_file(&supported_note, "# Public note");

        match symlink_file(&sensitive_target, &benign_link) {
            Ok(()) => {}
            Err(error) if symlink_unavailable(&error) => return,
            Err(error) => panic!(
                "create symlink {} -> {}: {error}",
                benign_link.display(),
                sensitive_target.display()
            ),
        }

        let error = collect_index_files(&benign_link, false)
            .expect_err("single symlink files must not be accepted for indexing");
        assert!(
            error.to_string().contains("symlink"),
            "unexpected error for symlink file: {error}"
        );

        let files =
            collect_index_files(directory.path(), true).expect("recursive collection succeeds");

        assert_eq!(
            relative_files(directory.path(), &files),
            vec![PathBuf::from("note.md")]
        );
    }

    #[test]
    fn recursive_collection_skips_unsupported_files_and_keeps_supported_markdown() {
        let directory = TestDirectory::create();
        write_file(&directory.path().join("note.md"), "# Normal note");
        write_file(
            &directory.path().join("docs/guide.markdown"),
            "# Normal guide",
        );
        write_file(
            &directory.path().join("docs/cache.sqlite"),
            "opaque database",
        );
        write_file(&directory.path().join("image.png"), "not text evidence");

        let files =
            collect_index_files(directory.path(), true).expect("recursive collection succeeds");

        assert_eq!(
            relative_files(directory.path(), &files),
            vec![
                PathBuf::from("docs/guide.markdown"),
                PathBuf::from("note.md"),
            ]
        );
    }

    #[test]
    fn recursive_collection_skips_excluded_entries_and_keeps_markdown() {
        let directory = TestDirectory::create();
        write_file(&directory.path().join("note.md"), "# Normal note");
        write_file(&directory.path().join("docs/guide.md"), "# Normal guide");
        write_file(&directory.path().join(".env.local"), "TOKEN=secret");
        write_file(&directory.path().join("cert.pem"), "private key");
        write_file(&directory.path().join("deploy.key"), "private key");
        write_file(&directory.path().join("secrets/passwords.md"), "secret");
        write_file(&directory.path().join(".ssh/config"), "Host secret");
        write_file(&directory.path().join(".gnupg/pubring.kbx"), "keyring");
        write_file(
            &directory.path().join("node_modules/package/index.js"),
            "module",
        );
        write_file(&directory.path().join("target/debug/app"), "binary");
        write_file(&directory.path().join("dist/bundle.js"), "bundle");
        write_file(&directory.path().join("build/output.o"), "object");

        let files =
            collect_index_files(directory.path(), true).expect("recursive collection succeeds");
        let relative_files = files
            .iter()
            .map(|path| {
                path.strip_prefix(directory.path())
                    .expect("collected file stays under root")
                    .to_path_buf()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            relative_files,
            vec![PathBuf::from("docs/guide.md"), PathBuf::from("note.md")]
        );
    }
    #[test]
    fn task_workspace_directory_is_deterministic_and_created() {
        let instance_dir = TestDirectory::create();
        let layout = InstanceLayout::for_root(instance_dir.path());
        let task_id = TaskId::new(42);

        assert_eq!(
            task_workspace_directory(&layout, task_id),
            layout.active_tasks_dir.join("task_42")
        );

        create_task_workspace_directories(&layout, task_id)
            .expect("initial workspace creation succeeds");
        create_task_workspace_directories(&layout, task_id)
            .expect("repeated workspace creation succeeds");

        let task_directory = task_workspace_directory(&layout, task_id);
        assert!(
            task_directory.is_dir(),
            "task workspace directory was not created"
        );
        for subdirectory in TASK_WORKSPACE_SUBDIRECTORIES {
            assert!(
                task_directory.join(subdirectory).is_dir(),
                "missing task workspace child directory: {subdirectory}"
            );
        }
    }

    #[test]
    fn is_db_locked_identifies_lock_and_busy_errors() {
        let locked = anyhow!("database is locked");
        assert!(is_db_locked(&locked));

        let busy = anyhow!("database is busy");
        assert!(is_db_locked(&busy));

        let locked_variant = anyhow!("SQLite error: locked");
        assert!(is_db_locked(&locked_variant));

        let other = anyhow!("file not found");
        assert!(!is_db_locked(&other));
    }

    // --- index_path recovery regression ---

    /// Verify that `recovery_inputs` — as called by `index_path` before
    /// `build_runtime` — correctly derives `ResumeParser` inputs from
    /// pending parsers and `StartFullTextIndex` inputs from pending
    /// full-text chunks, excluding artifacts that have a pending parser
    /// (whose resumed flow owns index dispatch).  This regression guards
    /// against the bug where CLI `index_path` started a fresh runtime but
    /// never queued durable pending work, causing an equal-hash artifact
    /// to be skipped and the CLI to wait until timeout.
    #[test]
    fn index_path_recovery_derives_pending_inputs_with_correct_filter() {
        use maestria_domain::{BlobId, ParserResult, ParserStarted, RegisterChunkInput};

        let mut state = KernelState::new();
        let artifact_a = ArtifactId::new(1); // has pending parser
        let artifact_b = ArtifactId::new(2); // has pending chunks only

        // artifact_a: ParserStarted replayed (pending parser)
        state.pending_parsers.insert(
            artifact_a,
            ParserStarted {
                artifact_id: artifact_a,
                title: "a.md".to_string(),
                source_path: "/tmp/a.md".to_string(),
                content_hash: "sha256:aaa".to_string(),
                blob_id: BlobId::new(100),
            },
        );

        // artifact_b: ParserCompleted created chunks but indexing not finished
        state
            .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id: artifact_b,
                title: "b.md".to_string(),
                source_path: "/tmp/b.md".to_string(),
                source_bytes: vec![1, 2, 3],
                content_hash: "sha256:bbb".to_string(),
            }))
            .expect("register artifact_b");
        state
            .apply_input(DomainInput::ParserCompleted(ParserResult {
                artifact_id: artifact_b,
                chunks: vec![RegisterChunkInput {
                    chunk_id: ChunkId::new(20),
                    artifact_id: artifact_b,
                    order: 0,
                    text: "hello".to_string(),
                }],
                cards: Vec::new(),
            }))
            .expect("parser completed for b");

        let recovery = maestria_daemon::recovery_inputs(&state);

        // artifact_a must be in resume_parsers
        assert_eq!(
            recovery.resume_parsers.len(),
            1,
            "one ResumeParser for artifact_a"
        );
        assert!(
            matches!(
                &recovery.resume_parsers[0],
                DomainInput::ResumeParser(r) if r.artifact_id == artifact_a
            ),
            "recovery.resume_parsers[0] must be ResumeParser for artifact_a"
        );

        // artifact_a must NOT appear in start_full_text (parser flow owns indexing)
        assert!(
            recovery
                .start_full_text
                .iter()
                .all(|input| !matches!(input, DomainInput::StartFullTextIndex(s) if s.artifact_id == artifact_a)),
            "artifact_a must be excluded from start_full_text"
        );

        // artifact_b must be in start_full_text
        assert_eq!(
            recovery.start_full_text.len(),
            1,
            "one StartFullTextIndex for artifact_b"
        );
        assert!(
            matches!(
                &recovery.start_full_text[0],
                DomainInput::StartFullTextIndex(s) if s.artifact_id == artifact_b
            ),
            "recovery.start_full_text[0] must be StartFullTextIndex for artifact_b"
        );
    }

    #[test]
    fn index_path_recovery_empty_when_no_pending_work() {
        let state = KernelState::new();
        let recovery = maestria_daemon::recovery_inputs(&state);
        assert!(recovery.resume_parsers.is_empty());
        assert!(recovery.start_full_text.is_empty());
    }
    // --- index_path recovery drain ---

    /// Verify that recovery artifact IDs are correctly extracted from
    /// both `resume_parsers` and `start_full_text` vectors, covering
    /// the two input kinds the drain loop must await.
    #[test]
    fn recovery_artifact_ids_covers_both_input_kinds() {
        use maestria_domain::{ParserStarted, StartFullTextIndex};

        let recovery = maestria_daemon::RecoveryInputs {
            resume_parsers: vec![DomainInput::ResumeParser(ParserStarted {
                artifact_id: ArtifactId::new(10),
                title: "a.md".to_string(),
                source_path: "/tmp/a.md".to_string(),
                content_hash: "sha256:aa".to_string(),
                blob_id: maestria_domain::BlobId::new(100),
            })],
            start_full_text: vec![
                DomainInput::StartFullTextIndex(StartFullTextIndex {
                    artifact_id: ArtifactId::new(20),
                }),
                DomainInput::StartFullTextIndex(StartFullTextIndex {
                    artifact_id: ArtifactId::new(30),
                }),
            ],
        };

        let ids: Vec<ArtifactId> = {
            let resume_ids = recovery.resume_parsers.iter().filter_map(|input| {
                if let DomainInput::ResumeParser(r) = input {
                    Some(r.artifact_id)
                } else {
                    None
                }
            });
            let ft_ids = recovery.start_full_text.iter().filter_map(|input| {
                if let DomainInput::StartFullTextIndex(s) = input {
                    Some(s.artifact_id)
                } else {
                    None
                }
            });
            resume_ids.chain(ft_ids).collect()
        };

        assert_eq!(ids.len(), 3, "three recovery artifact IDs");
        assert!(ids.contains(&ArtifactId::new(10)), "resume parser ID 10");
        assert!(ids.contains(&ArtifactId::new(20)), "start full-text ID 20");
        assert!(ids.contains(&ArtifactId::new(30)), "start full-text ID 30");
    }

    /// Verify that the recovery drain predicate — "all artifact IDs are
    /// Indexed" — correctly distinguishes terminal from non-terminal
    /// states.  This is the same check the drain loop uses each poll
    /// iteration before shutdown.
    #[test]
    fn recovery_drain_all_indexed_predicate() {
        use maestria_domain::ParserResult;

        let mut state = KernelState::new();
        let id_a = ArtifactId::new(1);
        let id_b = ArtifactId::new(2);

        // Register both artifacts through the full pipeline:
        // ArtifactDetected → ParserCompleted so they land in state.artifacts.
        for &id in &[id_a, id_b] {
            state
                .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
                    artifact_id: id,
                    title: format!("{id}.md"),
                    source_path: format!("/tmp/{id}.md"),
                    source_bytes: vec![id.value() as u8],
                    content_hash: format!("sha256:{id}"),
                }))
                .expect("register artifact");
            state
                .apply_input(DomainInput::ParserCompleted(ParserResult {
                    artifact_id: id,
                    chunks: Vec::new(),
                    cards: Vec::new(),
                }))
                .expect("parser completed");
        }

        // Predicate: all indexed
        let all_indexed = |state: &KernelState, ids: &[ArtifactId]| -> bool {
            ids.iter().all(|id| {
                state
                    .artifacts
                    .get(id)
                    .is_some_and(|a| a.index_status == IndexStatus::Indexed)
            })
        };

        // Neither indexed → false.
        assert!(!all_indexed(&state, &[id_a, id_b]));

        // Mark only id_a as Indexed → still false.
        state
            .artifacts
            .get_mut(&id_a)
            .expect("id_a must exist")
            .index_status = IndexStatus::Indexed;
        assert!(!all_indexed(&state, &[id_a, id_b]));

        // Mark id_b as Indexed → true.
        state
            .artifacts
            .get_mut(&id_b)
            .expect("id_b must exist")
            .index_status = IndexStatus::Indexed;
        assert!(all_indexed(&state, &[id_a, id_b]));

        // Empty list is vacuously true.
        assert!(all_indexed(&state, &[]));
    }

    /// Verify that `maestria_daemon::reconcile_projections` is callable
    /// from the CLI context and succeeds on a fresh in-memory store.
    /// The daemon crate's own `projection_recovery_tests` cover the full
    /// repair contract (missing artifact, chunk, card, evidence rows).
    /// This smoke test guards the CLI `index_path` path: the call site
    /// compiles against the CLI's dependency set and completes without
    /// error on a realistic kernel state.
    #[test]
    fn index_path_reconcile_projections_succeeds_in_cli_context() {
        let mut state = KernelState::new();
        let artifact_id = ArtifactId::new(100);

        state
            .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title: "cli-repair.md".to_string(),
                source_path: "/tmp/cli-repair.md".to_string(),
                source_bytes: vec![9, 8, 7],
                content_hash: "sha256:cli".to_string(),
            }))
            .expect("register artifact");

        let store = SqliteStore::in_memory().expect("open in-memory store");

        // Reconcile with the store — should succeed.
        maestria_daemon::reconcile_projections(&state, &store)
            .expect("reconcile should succeed on fresh store");

        // Reconcile again — must be idempotent.
        maestria_daemon::reconcile_projections(&state, &store)
            .expect("reconcile should be idempotent");
    }
}
