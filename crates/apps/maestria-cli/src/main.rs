use anyhow::Result;
use clap::{Parser as ClapParser, Subcommand};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

use maestria_blob_fs::FsBlobStore;
use maestria_domain::KernelState;
use maestria_governance::{AutonomyProfile, DefaultApprovalGate, DefaultRiskClassifier};
use maestria_parsers::ParserRegistry;
use maestria_ports::{InMemoryArtifactRepository, InMemoryEventLog, InMemoryHarnessAdapter};
use maestria_runtime::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_search_tantivy::TantivyFullTextIndex;
use std::sync::Arc;

#[derive(ClapParser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Start { instance_dir } => {
            info!("Starting Maestria in {:?}", instance_dir);
            std::fs::create_dir_all(&instance_dir)?;

            let blobs_dir = instance_dir.join("blobs");
            let index_dir = instance_dir.join("index");

            let blob_store = Arc::new(FsBlobStore::open(blobs_dir)?);
            let search_index = Arc::new(TantivyFullTextIndex::open(&index_dir)?);
            let parser = Arc::new(ParserRegistry::default());
            let event_log = Arc::new(InMemoryEventLog::default());
            let artifact_repo = Arc::new(InMemoryArtifactRepository::default());
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

            let _runtime_task = tokio::spawn(async move {
                runtime.run(input_rx).await;
            });

            // NOTE: Connect external sources to input_tx or use runtime.handle().
            // For now, just hold the process.
            tokio::signal::ctrl_c().await?;
            info!("Shutting down.");
        }
    }

    Ok(())
}
