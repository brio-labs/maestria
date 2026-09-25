use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum SearchApiCommands {
    /// Search a provider through a separate local process.
    Search {
        #[arg(long)]
        socket_path: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Read provider retrieval status through a search-scoped grant.
    Status {
        #[arg(long)]
        socket_path: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
    },
    /// Read approved roots, privacy exclusions and live indexing freshness (API v2).
    IndexingStatus {
        #[arg(long)]
        socket_path: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
    },
    /// Open bounded evidence through an evidence-scoped grant.
    OpenEvidence {
        #[arg(long)]
        socket_path: PathBuf,
        #[arg(long)]
        consumer_realm: String,
        #[arg(long)]
        credential_file: PathBuf,
        #[arg(long)]
        evidence_id: u64,
    },
}
