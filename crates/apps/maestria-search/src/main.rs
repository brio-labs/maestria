use std::io::Write;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand, ValueEnum};
use maestria_daemon::SearchApiOperation;
use maestria_domain::RealmId;
use maestria_governance::AutonomyProfile;

mod consumer;
mod owner;

const DEFAULT_INSTANCE_DIR: &str = ".maestria-dev";
const MAX_SEARCH_RESULTS: usize = 100;
const MAX_EVIDENCE_BYTES: usize = 65_536;
const MAX_GRANT_TTL_SECONDS: u64 = 365 * 24 * 60 * 60;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create an instance with explicitly approved read roots.
    Init {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
        #[arg(long = "read-root", value_delimiter = ',', num_args = 1.., required = true)]
        read_roots: Vec<PathBuf>,
    },
    /// Start the index-owning daemon in read-only mode, without a model client.
    Start {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
    },
    /// Administer roots and grants using the local instance-owner credential.
    Owner {
        #[command(subcommand)]
        command: OwnerCommands,
    },
    /// Search a provider through a separate local process.
    Search {
        #[command(flatten)]
        consumer: ConsumerApiArgs,
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Run a bounded local lexical passage and filename/path query for interactive clients.
    InteractiveSearch {
        #[command(flatten)]
        consumer: ConsumerApiArgs,
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Read provider retrieval status through a search-scoped grant.
    Status {
        #[command(flatten)]
        consumer: ConsumerApiArgs,
    },
    /// Read approved roots and live indexing freshness through API v2.
    IndexingStatus {
        #[command(flatten)]
        consumer: ConsumerApiArgs,
    },
    /// Open bounded evidence through an evidence-scoped grant.
    OpenEvidence {
        #[command(flatten)]
        consumer: ConsumerApiArgs,
        #[arg(long)]
        evidence_id: u64,
    },
}

#[derive(Subcommand)]
enum OwnerCommands {
    /// Administer explicitly approved live-search roots.
    Roots {
        #[command(subcommand)]
        command: RootCommands,
    },
    /// Administer external search-only grants.
    Grant {
        #[command(subcommand)]
        command: GrantCommands,
    },
}

#[derive(Subcommand)]
enum RootCommands {
    /// Show approved roots, indexed sources, exclusions, and watcher freshness.
    Status {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
    },
    /// Explicitly approve one directory for daemon-owned live indexing.
    Add {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
        root: PathBuf,
    },
    /// Revoke one approved directory from search and evidence scope.
    Remove {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
        root: PathBuf,
    },
}

#[derive(Subcommand)]
enum GrantCommands {
    /// Issue a search grant to an external realm and save its secret privately.
    CreateExternal {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
        #[arg(long, value_enum)]
        access: GrantAccess,
        #[arg(long, value_enum)]
        max_sensitivity: GrantSensitivity,
        #[arg(long, value_parser = parse_max_results)]
        max_results: usize,
        #[arg(long, value_parser = parse_max_evidence_bytes)]
        max_evidence_bytes: usize,
        /// Limit the grant to an explicitly approved read root; repeat as needed.
        #[arg(long = "read-root", value_delimiter = ',', num_args = 1..)]
        read_roots: Vec<PathBuf>,
        #[arg(long, default_value_t = 86_400, value_parser = parse_expiry_seconds)]
        expires_in_seconds: u64,
    },
    /// List current provider grants.
    List {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
    },
    /// Revoke a provider grant by its displayed digest.
    Revoke {
        #[arg(short, long, default_value = DEFAULT_INSTANCE_DIR)]
        instance_dir: PathBuf,
        grant_token_digest: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum GrantAccess {
    SearchOnly,
    SearchAndOpenEvidence,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum GrantSensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Args)]
struct ConsumerApiArgs {
    #[arg(long)]
    socket_path: PathBuf,
    #[arg(long)]
    consumer_realm: String,
    #[arg(long)]
    credential_file: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    dispatch(Cli::parse().command).await
}

async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Init {
            instance_dir,
            read_roots,
        } => initialize(instance_dir, read_roots),
        Commands::Start { instance_dir } => {
            maestria_daemon::run_instance_with_profile(instance_dir, AutonomyProfile::ReadOnly)
                .await
        }
        Commands::Owner { command } => owner::dispatch_owner(command).await,
        Commands::Search {
            consumer,
            query,
            limit,
        } => {
            consumer::run_consumer_request(consumer, SearchApiOperation::Search { query, limit })
                .await
        }
        Commands::InteractiveSearch {
            consumer,
            query,
            limit,
        } => {
            consumer::run_consumer_request(
                consumer,
                SearchApiOperation::InteractiveSearch { query, limit },
            )
            .await
        }
        Commands::Status { consumer } => {
            consumer::run_consumer_request(consumer, SearchApiOperation::Status).await
        }
        Commands::IndexingStatus { consumer } => {
            consumer::run_consumer_request(consumer, SearchApiOperation::IndexingStatus).await
        }
        Commands::OpenEvidence {
            consumer,
            evidence_id,
        } => {
            consumer::run_consumer_request(consumer, SearchApiOperation::Evidence { evidence_id })
                .await
        }
    }
}

fn initialize(instance_dir: PathBuf, read_roots: Vec<PathBuf>) -> Result<()> {
    let instance_dir = canonicalize_new_path(&instance_dir)?;
    let read_roots = read_roots
        .iter()
        .map(|root| owner::approved_root_argument(root))
        .collect::<Result<Vec<_>>>()?;
    let layout = maestria_daemon::prepare_instance_with_roots(instance_dir, read_roots)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "initialized {}", layout.root.display())?;
    writeln!(stdout, "manifest {}", layout.manifest_path.display())?;
    Ok(())
}

fn canonicalize_new_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("resolve new path {}", path.display()))?;
    let existing_ancestor = absolute
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .with_context(|| format!("find existing ancestor for {}", absolute.display()))?;
    let canonical_ancestor = existing_ancestor
        .canonicalize()
        .with_context(|| format!("canonicalize ancestor {}", existing_ancestor.display()))?;
    let unresolved = absolute
        .strip_prefix(existing_ancestor)
        .with_context(|| format!("resolve suffix for {}", absolute.display()))?;
    Ok(canonical_ancestor.join(unresolved))
}

fn parse_realm_id(value: String) -> Result<RealmId> {
    RealmId::try_from(value).map_err(|error| anyhow!(error))
}

fn parse_max_results(value: &str) -> std::result::Result<usize, &'static str> {
    let value = value
        .parse::<usize>()
        .map_err(|_| "maximum results must be a number")?;
    if !(1..=MAX_SEARCH_RESULTS).contains(&value) {
        return Err("maximum results must be 1..=100");
    }
    Ok(value)
}

fn parse_max_evidence_bytes(value: &str) -> std::result::Result<usize, &'static str> {
    let value = value
        .parse::<usize>()
        .map_err(|_| "maximum evidence bytes must be a number")?;
    if !(1..=MAX_EVIDENCE_BYTES).contains(&value) {
        return Err("maximum evidence bytes must be 1..=65536");
    }
    Ok(value)
}

fn parse_expiry_seconds(value: &str) -> std::result::Result<u64, &'static str> {
    let value = value
        .parse::<u64>()
        .map_err(|_| "grant expiry must be a number of seconds")?;
    if !(1..=MAX_GRANT_TTL_SECONDS).contains(&value) {
        return Err("grant expiry must be 1..=31536000 seconds");
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_grant_accepts_repeated_read_root_arguments() -> Result<()> {
        let args = vec![
            "maestria-search".to_string(),
            "owner".to_string(),
            "grant".to_string(),
            "create-external".to_string(),
            "--consumer-realm".to_string(),
            "a".repeat(64),
            "--credential-file".to_string(),
            "/tmp/grant-credential".to_string(),
            "--access".to_string(),
            "search-only".to_string(),
            "--max-sensitivity".to_string(),
            "public".to_string(),
            "--max-results".to_string(),
            "1".to_string(),
            "--max-evidence-bytes".to_string(),
            "1".to_string(),
            "--read-root".to_string(),
            "/tmp/approved-a".to_string(),
            "--read-root".to_string(),
            "/tmp/approved-b".to_string(),
        ];
        let cli = Cli::try_parse_from(args)?;
        let Commands::Owner {
            command:
                OwnerCommands::Grant {
                    command: GrantCommands::CreateExternal { read_roots, .. },
                },
        } = cli.command
        else {
            return Err(anyhow!("arguments did not select owner grant creation"));
        };

        assert_eq!(
            read_roots,
            vec![
                PathBuf::from("/tmp/approved-a"),
                PathBuf::from("/tmp/approved-b"),
            ]
        );
        Ok(())
    }
}
