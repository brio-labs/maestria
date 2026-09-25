use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum SearchCommands {
    /// Execute a search and print its durable plan and trace details
    Explain {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        task_id: Option<u64>,
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Show a persisted search trace by deterministic identifier
    Trace { trace_id: u64 },
    /// Compare two persisted search traces as an experiment pair
    Compare {
        experiment_a: u64,
        experiment_b: u64,
    },
    /// Query the persisted exact repository code index
    Code {
        #[command(subcommand)]
        command: CodeSearchCommands,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Inspect and administer explicitly approved live-search roots.
    Roots {
        #[command(subcommand)]
        command: SearchRootCommands,
    },
}

#[derive(Subcommand)]
pub enum SearchRootCommands {
    /// Show approved roots, indexed sources, formats, exclusions and watcher freshness.
    Status,
    /// Explicitly approve one directory for daemon-owned live indexing.
    Add { root: PathBuf },
    /// Revoke one approved directory immediately from search and evidence scope.
    Remove { root: PathBuf },
}

#[derive(Subcommand)]
pub enum CodeSearchCommands {
    /// Match repository symbols by name or qualified-name substring
    Symbol { pattern: String },
    /// Match repository symbols by source path substring
    Path { pattern: String },
    /// Match repository symbols and paths with a regular expression
    Regex { pattern: String },
    /// Match repository symbols whose doc comment contains the pattern
    Doc { pattern: String },
    /// Match repository symbols carrying a todo|fixme|hack|unsafe marker
    Markers { kind: String },
    /// Match symbols in files changed since a commit
    Changed {
        #[arg(long)]
        since: Option<String>,
    },
    /// Traverse bounded repository relations from a symbol seed
    Context {
        pattern: String,
        #[arg(short, long, default_value_t = 2)]
        depth: usize,
        #[arg(short, long, default_value_t = 64)]
        nodes: usize,
        #[arg(long, default_value = "both")]
        direction: String,
    },
    /// Resolve cross-file symbol references (inbound by default)
    References {
        pattern: String,
        #[arg(long)]
        direction: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum IndexCommands {
    /// List persisted index generations and lifecycle states
    Generations {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Build and persist exact Cargo metadata and Rust symbol records
    Repository {
        path: PathBuf,
        #[command(flatten)]
        selection: crate::commands::repository_index::RepositoryIndexArgs,
    },
}

#[derive(Subcommand)]
pub enum EvidenceCommands {
    /// Show evidence and validation coverage for a task
    Coverage {
        task_id: u64,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
}
