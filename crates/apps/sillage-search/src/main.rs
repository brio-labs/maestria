use std::io::Write;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand, ValueEnum};
use sillage_daemon::SearchApiOperation;
use sillage_domain::RealmId;
use sillage_governance::AutonomyProfile;

mod consumer;
mod owner;

const DEFAULT_INSTANCE_DIR: &str = ".sillage-dev";
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
        #[arg(long = "read-root", num_args = 1.., required = true)]
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
    /// Configure a reusable consumer connection without storing a bearer credential.
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
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
    /// Read the durable source-index revision through a search-scoped grant.
    SourceRevision {
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
enum ProfileCommands {
    /// Write a new non-secret profile for repeated consumer API calls.
    Configure {
        #[arg(long)]
        profile_file: PathBuf,
        #[arg(long)]
        socket_path: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
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
        #[arg(long = "read-root", num_args = 1..)]
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
    /// Load socket, realm, and separate credential-file paths from a profile.
    #[arg(long, conflicts_with = "socket_path")]
    profile: Option<PathBuf>,
    #[arg(long, required_unless_present = "profile", conflicts_with = "profile")]
    socket_path: Option<PathBuf>,
    #[arg(long, required_unless_present = "profile", conflicts_with = "profile")]
    consumer_realm: Option<String>,
    #[arg(long, required_unless_present = "profile", conflicts_with = "profile")]
    credential_file: Option<PathBuf>,
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
            sillage_daemon::run_instance_with_profile(instance_dir, AutonomyProfile::ReadOnly).await
        }
        Commands::Owner { command } => owner::dispatch_owner(command).await,
        Commands::Profile {
            command:
                ProfileCommands::Configure {
                    profile_file,
                    socket_path,
                    consumer_realm,
                    credential_file,
                },
        } => {
            consumer::create_consumer_profile(
                &profile_file,
                socket_path,
                consumer_realm,
                credential_file,
            )?;
            writeln!(
                std::io::stdout().lock(),
                "created consumer profile {}",
                profile_file.display()
            )?;
            Ok(())
        }
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
        Commands::SourceRevision { consumer } => {
            consumer::run_consumer_request(consumer, SearchApiOperation::SourceRevision).await
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
    let layout = sillage_daemon::prepare_instance_with_roots(instance_dir, read_roots)?;
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempProfile(PathBuf);

    impl TempProfile {
        fn fresh_path() -> PathBuf {
            static NEXT_PROFILE: AtomicUsize = AtomicUsize::new(0);
            std::env::temp_dir().join(format!(
                "sillage-search-profile-{}-{}.json",
                std::process::id(),
                NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
            ))
        }

        fn unused() -> Self {
            Self(Self::fresh_path())
        }

        fn create(contents: &[u8]) -> Result<Self> {
            let path = Self::fresh_path();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(contents)?;
            Ok(Self(path))
        }
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn valid_profile_json() -> String {
        format!(
            r#"{{
                "socket_path": "/run/sillage.sock",
                "consumer_realm": "{}",
                "credential_file": "/tmp/consumer.credential"
            }}"#,
            "a".repeat(64)
        )
    }

    fn resolve_profile_args(path: &Path) -> Result<consumer::ConsumerConnectionProfile> {
        consumer::resolve_consumer_connection(ConsumerApiArgs {
            profile: Some(path.to_path_buf()),
            socket_path: None,
            consumer_realm: None,
            credential_file: None,
        })
    }

    #[test]
    fn profile_configuration_writes_only_non_secret_metadata() -> Result<()> {
        let file = TempProfile::unused();
        let args: Vec<std::ffi::OsString> = vec![
            "sillage-search".into(),
            "profile".into(),
            "configure".into(),
            "--profile-file".into(),
            file.0.as_os_str().to_owned(),
            "--socket-path".into(),
            "/run/sillage.sock".into(),
            "--consumer-realm".into(),
            "a".repeat(64).into(),
            "--credential-file".into(),
            "/tmp/consumer.credential".into(),
        ];
        let cli = Cli::try_parse_from(args)?;
        let Commands::Profile {
            command:
                ProfileCommands::Configure {
                    profile_file,
                    socket_path,
                    consumer_realm,
                    credential_file,
                },
        } = cli.command
        else {
            return Err(anyhow!("arguments did not select profile configuration"));
        };
        consumer::create_consumer_profile(
            &profile_file,
            socket_path,
            consumer_realm,
            credential_file,
        )?;

        let contents = std::fs::read(&file.0)?;
        let value: serde_json::Value = serde_json::from_slice(&contents)?;
        let object = value
            .as_object()
            .ok_or_else(|| anyhow!("consumer profile was not a JSON object"))?;
        assert_eq!(object.len(), 3);
        let realm = "a".repeat(64);
        assert_eq!(object["socket_path"].as_str(), Some("/run/sillage.sock"));
        assert_eq!(object["consumer_realm"].as_str(), Some(realm.as_str()));
        assert_eq!(
            object["credential_file"].as_str(),
            Some("/tmp/consumer.credential")
        );
        assert!(!object.contains_key("credential"));
        Ok(())
    }

    #[test]
    fn consumer_profile_loads_validated_connection_values() -> Result<()> {
        let file = TempProfile::create(valid_profile_json().as_bytes())?;
        let profile = resolve_profile_args(&file.0)?;

        assert_eq!(profile.socket_path, PathBuf::from("/run/sillage.sock"));
        assert_eq!(profile.consumer_realm, "a".repeat(64));
        assert_eq!(
            profile.credential_file,
            PathBuf::from("/tmp/consumer.credential")
        );
        Ok(())
    }

    #[test]
    fn status_command_uses_consumer_profile() -> Result<()> {
        let file = TempProfile::create(valid_profile_json().as_bytes())?;
        let cli = Cli::try_parse_from([
            std::ffi::OsString::from("sillage-search"),
            std::ffi::OsString::from("status"),
            std::ffi::OsString::from("--profile"),
            file.0.as_os_str().to_owned(),
        ])?;
        let Commands::Status { consumer } = cli.command else {
            return Err(anyhow!("arguments did not select consumer status"));
        };
        let connection = consumer::resolve_consumer_connection(consumer)?;

        assert_eq!(connection.socket_path, PathBuf::from("/run/sillage.sock"));
        assert_eq!(connection.consumer_realm, "a".repeat(64));
        assert_eq!(
            connection.credential_file,
            PathBuf::from("/tmp/consumer.credential")
        );
        Ok(())
    }

    #[test]
    fn consumer_profile_rejects_an_embedded_credential() -> Result<()> {
        let secret = "bearer-secret-must-not-be-loaded";
        let contents = format!(
            r#"{{
                "socket_path": "/run/sillage.sock",
                "consumer_realm": "{}",
                "credential_file": "/tmp/consumer.credential",
                "credential": "{secret}"
            }}"#,
            "a".repeat(64)
        );
        let file = TempProfile::create(contents.as_bytes())?;
        let error =
            resolve_profile_args(&file.0).expect_err("embedded credentials must be rejected");

        let error_message = format!("{error:#}");
        assert!(error_message.contains("credential"));
        assert!(!error_message.contains(secret));
        Ok(())
    }

    #[test]
    fn consumer_profile_rejects_invalid_realm_and_oversized_input() -> Result<()> {
        let invalid_realm = r#"{
            "socket_path": "/run/sillage.sock",
            "consumer_realm": "",
            "credential_file": "/tmp/consumer.credential"
        }"#;
        let file = TempProfile::create(invalid_realm.as_bytes())?;
        assert!(resolve_profile_args(&file.0).is_err());

        let oversized = vec![b' '; (consumer::MAX_CONSUMER_PROFILE_BYTES + 1) as usize];
        let file = TempProfile::create(&oversized)?;
        let error = resolve_profile_args(&file.0).expect_err("oversized profiles must be rejected");
        assert!(error.to_string().contains("maximum size"));
        Ok(())
    }

    #[test]
    fn direct_consumer_arguments_remain_available() -> Result<()> {
        let cli = Cli::try_parse_from(vec![
            "sillage-search".to_string(),
            "status".to_string(),
            "--socket-path".to_string(),
            "/run/sillage.sock".to_string(),
            "--consumer-realm".to_string(),
            "a".repeat(64),
            "--credential-file".to_string(),
            "/tmp/consumer.credential".to_string(),
        ])?;
        let Commands::Status { consumer } = cli.command else {
            return Err(anyhow!("arguments did not select consumer status"));
        };
        let connection = consumer::resolve_consumer_connection(consumer)?;
        assert_eq!(connection.socket_path, PathBuf::from("/run/sillage.sock"));
        Ok(())
    }

    #[test]
    fn owner_grant_accepts_repeated_read_root_arguments() -> Result<()> {
        let args = vec![
            "sillage-search".to_string(),
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
            "/tmp/approved-a, reviewed".to_string(),
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
                PathBuf::from("/tmp/approved-a, reviewed"),
                PathBuf::from("/tmp/approved-b"),
            ]
        );
        Ok(())
    }

    #[test]
    fn initialization_preserves_commas_and_spaces_in_approved_root() -> Result<()> {
        let root = "/tmp/document notes, reviewed";
        let cli = Cli::try_parse_from(["sillage-search", "init", "--read-root", root])?;
        let Commands::Init { read_roots, .. } = cli.command else {
            return Err(anyhow!("arguments did not select initialization"));
        };
        assert_eq!(read_roots, vec![PathBuf::from(root)]);
        Ok(())
    }
}
