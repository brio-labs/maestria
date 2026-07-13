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
pub enum TaskCommands {
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
    /// Link an existing evidence record to a task
    AddEvidence {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        task_id: u64,
        #[arg(long)]
        evidence_id: u64,
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
