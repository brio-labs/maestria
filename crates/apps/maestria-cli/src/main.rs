use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use clap::{Parser as ClapParser, Subcommand};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{
    CorePorts, CoreServices, IngestFileInput, InitInstanceInput, InstanceLayout, InstanceService,
    OpenChunkEvidenceInput, OpenEvidenceInput, SearchInput,
};
use maestria_domain::{ChunkId, EvidenceId, EvidenceKind, KernelState, LogicalTick};
use maestria_governance::{AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier};
use maestria_parsers::ParserRegistry;
use maestria_ports::{EventFilter, InMemoryHarnessAdapter};
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { instance_dir } => init_instance(instance_dir)?,
        Commands::Index {
            instance_dir,
            path,
            recursive,
        } => index_path(instance_dir, path, recursive)?,
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
        Commands::Start { instance_dir } => start(instance_dir).await?,
    }

    Ok(())
}

fn init_instance(instance_dir: PathBuf) -> Result<()> {
    let plan = InstanceService::init_instance(InitInstanceInput { root: instance_dir })?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    println!("initialized {}", plan.layout.root.display());
    println!("manifest {}", plan.manifest_path.display());
    Ok(())
}

fn index_path(instance_dir: PathBuf, path: PathBuf, recursive: bool) -> Result<()> {
    let layout = ensure_instance(instance_dir)?;
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

    let files = collect_index_files(&path, recursive)?;
    if files.is_empty() {
        return Err(anyhow!(
            "no files selected for indexing at {}",
            path.display()
        ));
    }

    for file in files {
        let bytes = fs::read(&file)?;
        let output = core.ingest_file_from_bytes(IngestFileInput {
            path: file.clone(),
            bytes,
            observed_at: LogicalTick::new(1),
            artifact_id: None,
        })?;
        println!(
            "indexed artifact={} chunks={} evidence={} path={}",
            output.artifact.id,
            output.chunks.len(),
            output.evidence.len(),
            file.display()
        );
    }

    Ok(())
}

fn search(instance_dir: PathBuf, query: String, limit: usize) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
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
    let layout = InstanceLayout::for_root(instance_dir);
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
    let sqlite_store = SqliteStore::open(&layout.database_path)?;
    let events = maestria_ports::EventLog::scan(&sqlite_store, EventFilter { artifact_id: None })?;
    println!("instance {}", layout.root.display());
    println!("database {}", layout.database_path.display());
    println!("full_text_index {}", layout.full_text_index_dir.display());
    println!("events {}", events.len());
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

async fn start(instance_dir: PathBuf) -> Result<()> {
    let layout = ensure_instance(instance_dir)?;
    info!("Starting Maestria in {:?}", layout.root);

    let blob_store = Arc::new(FsBlobStore::open(&layout.blobs_dir)?);
    let search_index = Arc::new(TantivyFullTextIndex::open(&layout.full_text_index_dir)?);
    let parser = Arc::new(ParserRegistry::with_defaults());
    let sqlite_store = Arc::new(SqliteStore::open(&layout.database_path)?);
    let event_log = sqlite_store.clone();
    let artifact_repo = sqlite_store.clone();
    let harness = Arc::new(InMemoryHarnessAdapter::default());

    let adapters = Adapters {
        event_log,
        blob_store,
        search_index,
        parser,
        harness,
        artifact_repo,
    };

    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };

    let config = RuntimeConfig {
        profile: AutonomyProfile::ReadOnly,
        ..Default::default()
    };

    let state = KernelState::new();
    let (runtime, input_rx) = MaestriaRuntime::new(config, state, adapters, governance);

    info!("Maestria runtime started.");
    let shutdown_token = tokio_util::sync::CancellationToken::new();
    let token_clone = shutdown_token.clone();
    let _runtime_task = tokio::spawn(async move {
        runtime.run(input_rx, token_clone).await;
    });

    tokio::signal::ctrl_c().await?;
    info!("Shutting down Maestria...");
    shutdown_token.cancel();
    info!("Shutting down.");
    Ok(())
}

fn ensure_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let plan = InstanceService::init_instance(InitInstanceInput { root: instance_dir })?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    if !plan.manifest_path.exists() {
        fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    }
    Ok(plan.layout)
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
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".env" | ".ssh" | ".gnupg" | "secrets" | "node_modules" | "target" | "dist" | "build"
        ) || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
    })
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
        Some("md" | "markdown" | "txt" | "text" | "rs")
    )
}

fn source_label(evidence: &maestria_domain::Evidence) -> String {
    match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
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
}
