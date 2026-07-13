use anyhow::Result;
use clap::Parser;
use maestria_domain::{HarnessRunId, MaestriaEffect, QueryHarnessRequest, ScopeId};
use maestria_governance::{
    ApprovalGate, ApprovalRequest, AutonomyProfile, DefaultApprovalGate, PolicyDecision, Scope,
    ScopeGuard,
};
use maestria_harness::LocalShellHarnessAdapter;
use maestria_ports::{HarnessAdapter, HarnessCommandClass, HarnessRequest};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(author, version, about = "Maestria Local Harness CLI")]
struct Cli {
    #[arg(short, long)]
    command: String,

    #[arg(short, long, default_value = ".")]
    working_directory: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let adapter = LocalShellHarnessAdapter;

    let working_directory = std::fs::canonicalize(&cli.working_directory)?;
    let scope = Scope::new(
        vec![working_directory.clone()],
        vec![],
        vec!["shell".into()],
        vec![],
        false,
    );
    let guard = ScopeGuard::new(scope.clone());

    // Governance authorization — decide before spawn
    if !scope.harness_allowed("shell") {
        println!("Governance: Denied. Shell harness not permitted by scope.");
        return Ok(());
    }
    let gate = DefaultApprovalGate;
    let profile = AutonomyProfile::TrustedWorkspace;
    let effect = MaestriaEffect::QueryHarness(QueryHarnessRequest {
        run_id: HarnessRunId::new(1),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: ScopeId::new(1),
        approval_id: None,
        command: cli.command.clone(),
    });
    let decision = gate.decide(&ApprovalRequest {
        effect: &effect,
        profile,
        scope: &guard,
    });
    match decision.decision {
        PolicyDecision::Deny { reason } => {
            println!("Governance: Denied. {reason}");
            return Ok(());
        }
        PolicyDecision::RequireApproval { reason } => {
            println!("Governance: Requires approval. {reason}");
            return Ok(());
        }
        PolicyDecision::Allow => {}
    }
    println!("Governance: Approved. Risk: {:?}", decision.risk);

    let request = HarnessRequest {
        run_id: HarnessRunId::new(1),
        command: cli.command.clone(),
        working_directory,
        duration_budget: Duration::from_secs(300),
        class: HarnessCommandClass::Shell,
        readable_roots: scope.readable_roots().to_vec(),
        blocked_paths: vec![],
        blocked_patterns: vec![],
    };

    let outcome = adapter.execute(request).await?;

    println!("Exit code: {}", outcome.exit_code);
    println!("Duration: {:?}", outcome.duration);

    if !outcome.stdout.is_empty() {
        println!(
            "--- STDOUT ---\n{}",
            String::from_utf8_lossy(&outcome.stdout)
        );
    }

    if !outcome.stderr.is_empty() {
        println!(
            "--- STDERR ---\n{}",
            String::from_utf8_lossy(&outcome.stderr)
        );
    }

    Ok(())
}
