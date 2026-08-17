use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use maestria_domain::TaskPriority;

#[path = "cli_types/parsers.rs"]
mod parsers;

#[derive(ClapParser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a local Maestria instance layout
    Init {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long = "read-root", value_delimiter = ',', num_args = 1..)]
        read_roots: Vec<PathBuf>,
    },
    /// Index one local file, files under a directory, or inspect index generations
    Index {
        #[command(subcommand)]
        command: Option<IndexCommands>,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        path: Option<PathBuf>,
        #[arg(short, long)]
        recursive: bool,
        #[arg(long, help = "Skip files larger than N bytes; 0 disables")]
        max_file_bytes: Option<u64>,
        #[arg(long, help = "Skip generated asset dumps (single-extension dumps)")]
        skip_generated: bool,
        #[arg(long, help = "Skip minified single-line bundles")]
        skip_minified: bool,
        #[arg(long, help = "Accept every directory prompt (non-interactive)")]
        yes: bool,
        #[arg(
            long,
            help = "Write the approved selection to system/index-selection.json"
        )]
        save_selection: bool,
    },
    Search {
        #[command(subcommand)]
        command: Option<SearchCommands>,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        /// Associate direct search with an optional task.
        #[arg(long)]
        task_id: Option<u64>,
        query: Option<String>,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Resolve typed source evidence without launching external programs
    OpenEvidence {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long, conflicts_with = "chunk_id")]
        evidence_id: Option<u64>,
        #[arg(long, conflicts_with = "evidence_id")]
        chunk_id: Option<u64>,
    },
    /// Inspect task evidence coverage
    Evidence {
        #[command(subcommand)]
        command: EvidenceCommands,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
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
        /// Autonomy profile for the daemon runtime: read-only (default,
        /// denies medium-risk effects such as vector indexing, validations,
        /// and graph updates) or trusted-workspace (allows them; required
        /// when the daemon is the primary ingestion path).
        #[arg(long, value_enum, default_value = "read-only")]
        profile: DaemonProfile,
    },
    /// Launch the local authenticated Studio frontend
    Studio {
        /// Instance containing the daemon socket, token, and Studio configuration.
        #[arg(short = 'i', long = "instance-dir", default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        /// Do not open the printed Studio URL in the default browser.
        #[arg(long)]
        no_open: bool,
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
    /// Approval request management
    Approval {
        #[command(subcommand)]
        command: ApprovalCommands,
    },
    /// Manage explicit local realm-federation grants and reads
    Realm {
        #[command(subcommand)]
        command: RealmCommands,
    },
    /// Learned-sparse promotion record management
    Promotion {
        #[command(subcommand)]
        command: PromotionCommands,
    },
}

#[derive(Subcommand)]
pub enum PromotionCommands {
    /// Set (or replace) the instance promotion record from a JSON file
    Set {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(
            long,
            help = "Path to a serialized LearnedSparsePromotionRecord JSON file"
        )]
        record: PathBuf,
    },
    /// Remove the promotion record and restore the lexical/hybrid route
    Remove {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Print the stored promotion record (or a no-record notice)
    Show {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum SearchCommands {
    /// Execute a search and print its durable plan and trace details
    Explain {
        #[arg(long)]
        task_id: Option<u64>,
        query: String,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Show a persisted search trace by deterministic identifier
    Trace {
        trace_id: u64,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Compare two persisted search traces as an experiment pair
    Compare {
        experiment_a: u64,
        experiment_b: u64,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Query the persisted exact repository code index
    Code {
        #[command(subcommand)]
        command: CodeSearchCommands,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
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

#[derive(Subcommand)]
/// Task workflow commands
pub enum TaskCommands {
    /// Create a new task in persisted task state
    Start {
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
    /// Link an existing evidence record to a task
    AddEvidence {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        task_id: u64,
        #[arg(long)]
        evidence_id: u64,
    },
    /// Start validation for a task from a known task id
    RequestValidation {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        task_id: u64,
    },
    /// Complete a validating task from a recorded validation report
    Complete {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        task_id: u64,
        #[arg(long)]
        report_id: u64,
    },
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// List persisted memory candidates
    Candidates {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Propose a new memory candidate backed by evidence
    Propose {
        #[arg(short, long)]
        text: String,
        #[arg(short = 'e', long, value_delimiter = ',', num_args = 1..)]
        evidence_id: Vec<u64>,
        #[arg(short, long, value_parser = clap::value_parser!(u16).range(0..=1000))]
        confidence_milli: u16,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Promote a memory candidate through governance-gated approval
    Promote {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(short = 'c', long)]
        candidate_id: u64,
        /// User approval for this promotion request
        #[arg(long)]
        approve: bool,
    },
}
#[derive(Subcommand)]
pub enum ApprovalCommands {
    /// List pending approval requests
    List {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
    /// Resolve an approval request
    Resolve {
        /// Approval request ID
        id: u64,
        /// Approve the request
        #[arg(long, conflicts_with = "deny")]
        approve: bool,
        /// Deny the request
        #[arg(long, conflicts_with = "approve")]
        deny: bool,
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
    },
}

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

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliTaskPriority {
    Low,
    Normal,
    High,
}

/// Daemon autonomy profiles selectable via `maestria start --profile`.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum DaemonProfile {
    ReadOnly,
    TrustedWorkspace,
}

impl DaemonProfile {
    pub fn governance_profile(self) -> maestria_governance::AutonomyProfile {
        match self {
            Self::ReadOnly => maestria_governance::AutonomyProfile::ReadOnly,
            Self::TrustedWorkspace => maestria_governance::AutonomyProfile::TrustedWorkspace,
        }
    }
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
