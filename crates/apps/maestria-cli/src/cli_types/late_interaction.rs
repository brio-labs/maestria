use std::path::PathBuf;

use clap::Subcommand;

/// Late-interaction report and rollback management.
#[derive(Subcommand)]
pub enum LateInteractionCommands {
    /// Print the latest persisted late-interaction report for a stage.
    Show {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        /// One of `stage-a`, `stage-b`, or `promotion`.
        #[arg(long, default_value = "stage-a")]
        stage: String,
    },
    /// Validate and store a real Stage A promotion record.
    Set {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        record: PathBuf,
    },
    /// Remove one persisted late-interaction promotion record.
    Rollback {
        #[arg(short, long, default_value = ".maestria-dev")]
        instance_dir: PathBuf,
        #[arg(long)]
        evaluation_id: String,
    },
}
