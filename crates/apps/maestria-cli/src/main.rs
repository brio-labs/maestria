mod cli_types;
mod commands;
mod helpers;

#[cfg(test)]
mod tests;

use anyhow::Result;
use clap::Parser as ClapParser;
use cli_types::{Cli, Commands, MemoryCommands, TaskCommands};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            instance_dir,
            read_roots,
        } => commands::init::run(instance_dir, read_roots)?,
        Commands::Index {
            instance_dir,
            path,
            recursive,
        } => commands::index::run(instance_dir, path, recursive).await?,
        Commands::Search {
            instance_dir,
            query,
            limit,
        } => commands::search::run(instance_dir, query, limit)?,
        Commands::OpenEvidence {
            instance_dir,
            evidence_id,
            chunk_id,
        } => commands::evidence::run(instance_dir, evidence_id, chunk_id)?,
        Commands::Status { instance_dir } => commands::status::run(instance_dir)?,
        Commands::Doctor { instance_dir } => commands::doctor::run(instance_dir)?,
        Commands::Start { instance_dir } => maestria_daemon::run_instance(instance_dir).await?,
        Commands::Task { command } => match command {
            TaskCommands::Start {
                title,
                instance_dir,
                priority,
                artifact_id,
            } => {
                commands::task::run_start(instance_dir, title, priority, artifact_id).await?;
            }
            TaskCommands::Show {
                instance_dir,
                task_id,
            } => {
                commands::task::run_show(instance_dir, task_id)?;
            }
            TaskCommands::AddEvidence {
                instance_dir,
                task_id,
                evidence_id,
            } => {
                commands::task::run_add_evidence(instance_dir, task_id, evidence_id).await?;
            }
        },
        Commands::Memory { command } => match command {
            MemoryCommands::Candidates {
                instance_dir,
                limit,
            } => commands::memory::run(instance_dir, limit)?,
        },
    }

    Ok(())
}
