mod cli_types;
mod commands;
mod helpers;

#[cfg(test)]
mod tests;

use anyhow::Result;
use clap::Parser as ClapParser;
use cli_types::{
    ApprovalCommands, Cli, CodeSearchCommands, Commands, EvidenceCommands, IndexCommands,
    MemoryCommands, SearchCommands, TaskCommands,
};

fn resolve_nested_instance_dir(
    outer: std::path::PathBuf,
    inner: std::path::PathBuf,
) -> std::path::PathBuf {
    if inner == std::path::Path::new(".maestria-dev") {
        outer
    } else {
        inner
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    dispatch(Cli::parse().command).await
}

async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Init {
            instance_dir,
            read_roots,
        } => commands::init::run(instance_dir, read_roots)?,
        Commands::Index {
            command,
            instance_dir,
            path,
            recursive,
        } => dispatch_index(command, instance_dir, path, recursive).await?,
        Commands::Search {
            command,
            instance_dir,
            query,
            task_id,
            limit,
        } => dispatch_search(command, instance_dir, query, task_id, limit).await?,
        Commands::OpenEvidence {
            instance_dir,
            evidence_id,
            chunk_id,
        } => commands::evidence::run(instance_dir, evidence_id, chunk_id)?,
        Commands::Evidence {
            command,
            instance_dir,
        } => dispatch_evidence(command, instance_dir)?,
        Commands::Status { instance_dir } => commands::status::run(instance_dir)?,
        Commands::Doctor { instance_dir } => commands::doctor::run(instance_dir)?,
        Commands::Start { instance_dir } => maestria_daemon::run_instance(instance_dir).await?,
        Commands::Task { command } => dispatch_task(command).await?,
        Commands::Memory { command } => dispatch_memory(command).await?,
        Commands::Approval { command } => dispatch_approval(command).await?,
    }
    Ok(())
}

async fn dispatch_index(
    command: Option<IndexCommands>,
    instance_dir: std::path::PathBuf,
    path: Option<std::path::PathBuf>,
    recursive: bool,
) -> Result<()> {
    match command {
        Some(IndexCommands::Generations {
            instance_dir: nested_instance_dir,
        }) => commands::observability::run_index_generations(resolve_nested_instance_dir(
            instance_dir,
            nested_instance_dir,
        )),
        Some(IndexCommands::Repository { path }) => {
            commands::code_intel::run_index(instance_dir, path)
        }
        None => {
            let path = path.ok_or_else(|| anyhow::anyhow!("index requires a path"))?;
            commands::index::run(instance_dir, path, recursive).await
        }
    }
}

async fn dispatch_search(
    command: Option<SearchCommands>,
    instance_dir: std::path::PathBuf,
    query: Option<String>,
    task_id: Option<u64>,
    limit: usize,
) -> Result<()> {
    match command {
        Some(SearchCommands::Explain {
            query,
            instance_dir: nested_instance_dir,
            limit,
            task_id,
        }) => {
            let instance_dir = resolve_nested_instance_dir(instance_dir, nested_instance_dir);
            commands::observability::run_search_explain(instance_dir, task_id, query, limit).await
        }
        Some(SearchCommands::Trace {
            trace_id,
            instance_dir: nested_instance_dir,
        }) => {
            let instance_dir = resolve_nested_instance_dir(instance_dir, nested_instance_dir);
            commands::observability::run_search_trace(instance_dir, trace_id)
        }
        Some(SearchCommands::Compare {
            experiment_a,
            experiment_b,
            instance_dir: nested_instance_dir,
        }) => {
            let instance_dir = resolve_nested_instance_dir(instance_dir, nested_instance_dir);
            commands::observability::run_search_compare(instance_dir, experiment_a, experiment_b)
        }
        Some(SearchCommands::Code {
            command,
            instance_dir: nested_instance_dir,
            limit,
        }) => {
            let instance_dir = resolve_nested_instance_dir(instance_dir, nested_instance_dir);
            let query = match command {
                CodeSearchCommands::Symbol { pattern } => {
                    maestria_code_intel::CodeQuery::Symbol { pattern }
                }
                CodeSearchCommands::Path { pattern } => {
                    maestria_code_intel::CodeQuery::Path { pattern }
                }
                CodeSearchCommands::Regex { pattern } => {
                    maestria_code_intel::CodeQuery::Regex { pattern }
                }
            };
            commands::code_intel::run_search(instance_dir, query, limit)
        }
        None => {
            let query = query.ok_or_else(|| anyhow::anyhow!("search requires a query"))?;
            commands::search::run(instance_dir, task_id, query, limit).await
        }
    }
}

fn dispatch_evidence(command: EvidenceCommands, instance_dir: std::path::PathBuf) -> Result<()> {
    match command {
        EvidenceCommands::Coverage {
            task_id,
            instance_dir: nested_instance_dir,
        } => commands::observability::run_evidence_coverage(
            resolve_nested_instance_dir(instance_dir, nested_instance_dir),
            task_id,
        ),
    }
}

async fn dispatch_task(command: TaskCommands) -> Result<()> {
    match command {
        TaskCommands::Start {
            title,
            instance_dir,
            priority,
            artifact_id,
        } => commands::task::run_start(instance_dir, title, priority, artifact_id).await,
        TaskCommands::Show {
            instance_dir,
            task_id,
        } => commands::task::run_show(instance_dir, task_id),
        TaskCommands::AddEvidence {
            instance_dir,
            task_id,
            evidence_id,
        } => commands::task::run_add_evidence(instance_dir, task_id, evidence_id).await,
        TaskCommands::RequestValidation {
            instance_dir,
            task_id,
        } => commands::task::run_request_validation(instance_dir, task_id).await,
        TaskCommands::Complete {
            instance_dir,
            task_id,
            report_id,
        } => commands::task::run_complete(instance_dir, task_id, report_id).await,
    }
}

async fn dispatch_memory(command: MemoryCommands) -> Result<()> {
    match command {
        MemoryCommands::Candidates {
            instance_dir,
            limit,
        } => commands::memory::run(instance_dir, limit),
        MemoryCommands::Propose {
            text,
            evidence_id,
            confidence_milli,
            instance_dir,
        } => commands::memory::run_propose(instance_dir, text, evidence_id, confidence_milli).await,
        MemoryCommands::Promote {
            instance_dir,
            candidate_id,
            approve,
        } => commands::memory::run_promote(instance_dir, candidate_id, approve).await,
    }
}

async fn dispatch_approval(command: ApprovalCommands) -> Result<()> {
    match command {
        ApprovalCommands::List { instance_dir } => commands::approval::run_list(instance_dir),
        ApprovalCommands::Resolve {
            id,
            approve,
            deny,
            instance_dir,
        } => {
            let approved = if approve {
                true
            } else if deny {
                false
            } else {
                anyhow::bail!("must specify either --approve or --deny");
            };
            commands::approval::run_resolve(instance_dir, id, approved).await
        }
    }
}
