use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use super::parsers;

#[derive(Subcommand)]
pub enum RealmCommands {
    /// Explicitly migrate a schema-v1 instance manifest to schema v2
    Migrate {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Print this instance's stable realm identity
    Identity {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Provider-owned realm grant administration
    Grant {
        #[command(subcommand)]
        command: RealmGrantCommands,
    },
    /// Search a bound provider through the consumer daemon
    Search {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        provider_realm: String,
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Open bounded evidence from a bound provider through the consumer daemon
    OpenEvidence {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        provider_realm: String,
        #[arg(long)]
        evidence_id: u64,
    },
}

#[derive(Subcommand)]
pub enum RealmGrantCommands {
    /// Issue a grant and install its private consumer binding
    Create {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        consumer_instance: PathBuf,
        #[arg(long, value_enum)]
        access: CliRealmGrantAccess,
        #[arg(long, value_enum)]
        max_sensitivity: CliRealmGrantSensitivity,
        #[arg(long, value_parser = parsers::parse_federated_results)]
        max_results: usize,
        #[arg(long, value_parser = parsers::parse_federated_evidence_bytes)]
        max_evidence_bytes: usize,
        #[arg(long, default_value_t = 86_400)]
        expires_in_seconds: u64,
    },
    /// Issue a search grant to an external consumer realm and save its secret privately.
    CreateExternal {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
        #[arg(long, value_enum)]
        access: CliRealmGrantAccess,
        #[arg(long, value_enum)]
        max_sensitivity: CliRealmGrantSensitivity,
        #[arg(long, value_parser = parsers::parse_federated_results)]
        max_results: usize,
        #[arg(long, value_parser = parsers::parse_federated_evidence_bytes)]
        max_evidence_bytes: usize,
        #[arg(long, default_value_t = 86_400)]
        expires_in_seconds: u64,
    },
    /// List current provider grants
    List {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Revoke a provider grant by its displayed digest
    Revoke {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        grant_token_digest: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliRealmGrantAccess {
    SearchOnly,
    SearchAndOpenEvidence,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliRealmGrantSensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}
