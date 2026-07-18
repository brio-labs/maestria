use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use maestria_domain::TaskPriority;

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
}

#[derive(Subcommand)]
pub enum IndexCommands {
    /// List persisted index generations and lifecycle states
    Generations {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
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

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliTaskPriority {
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
