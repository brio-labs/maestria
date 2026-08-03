use anyhow::Result;
use clap::Parser;
use maestria_domain::{
    HarnessExecution, HarnessRunId, MaestriaEffect, QueryHarnessRequest, ScopeId,
};
use maestria_governance::{
    ApprovalGate, ApprovalRequest, AutonomyProfile, ClassifyRisk, DefaultApprovalGate,
    DefaultRiskClassifier, PolicyDecision, PrivacyExclusions, Scope, ScopeGuard,
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

fn privacy_patterns() -> Vec<String> {
    let privacy = PrivacyExclusions::default();
    let mut patterns: Vec<String> = privacy.sensitive_names().to_vec();
    patterns.extend(
        privacy
            .sensitive_extensions()
            .iter()
            .map(|ext| format!("*.{ext}")),
    );
    patterns
}

fn enforce_policy_decision(decision: &PolicyDecision) -> Result<()> {
    match decision {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => anyhow::bail!("Governance: Denied. {reason}"),
        PolicyDecision::RequireApproval { reason } => {
            anyhow::bail!("Governance: Requires approval. {reason}")
        }
    }
}
fn normalize_exit_code(exit_code: i32) -> i32 {
    if exit_code <= 0 {
        1
    } else {
        exit_code.min(u8::MAX as i32)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let working_directory = std::fs::canonicalize(&cli.working_directory)?;
    let scope = Scope::new(
        vec![working_directory.clone()],
        vec![],
        vec!["shell".into()],
        vec![],
        false,
    );
    let guard = ScopeGuard::new(scope.clone());

    // Governance authorization — decide before execution.
    if !scope.harness_allowed("shell") {
        println!("Governance: Denied. Shell harness not permitted by scope.");
        return Ok(());
    }
    let gate = DefaultApprovalGate;
    let profile = AutonomyProfile::TrustedWorkspace;
    let effect = MaestriaEffect::QueryHarness(QueryHarnessRequest {
        run_id: HarnessRunId::new(1),
        task_id: None,
        execution: HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: ScopeId::new(1),
        command: cli.command.clone(),
    });
    let risk = DefaultRiskClassifier.classify(&effect, &guard);
    let decision = gate.decide(&ApprovalRequest {
        effect: &effect,
        profile,
        risk,
        scope: &guard,
    });
    enforce_policy_decision(&decision.decision)?;
    println!("Governance: Approved. Risk: {:?}", decision.risk);

    let adapter = LocalShellHarnessAdapter;
    let request = HarnessRequest {
        run_id: HarnessRunId::new(1),
        command: cli.command.clone(),
        working_directory,
        duration_budget: Duration::from_secs(300),
        class: HarnessCommandClass::Shell,
        readable_roots: scope.readable_roots().to_vec(),
        blocked_paths: vec![],
        blocked_patterns: privacy_patterns(),
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
        eprintln!(
            "--- STDERR ---\n{}",
            String::from_utf8_lossy(&outcome.stderr)
        );
    }

    if outcome.exit_code != 0 {
        std::process::exit(normalize_exit_code(outcome.exit_code));
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_policy_decision_allows_allowing_decision() {
        let decision = PolicyDecision::Allow;
        assert!(enforce_policy_decision(&decision).is_ok());
    }

    #[test]
    fn normalize_exit_code_preserves_platform_exit_range() {
        assert_eq!(normalize_exit_code(1), 1);
        assert_eq!(normalize_exit_code(i32::MAX), i32::from(u8::MAX));
        assert_eq!(normalize_exit_code(-1), 1);
        assert_eq!(normalize_exit_code(0), 1);
    }

    #[test]
    fn enforce_policy_decision_denies_with_error() -> Result<()> {
        let err = match enforce_policy_decision(&PolicyDecision::Deny {
            reason: "blocked command".into(),
        }) {
            Err(error) => error,
            Ok(()) => return Err(anyhow::anyhow!("denied policy unexpectedly allowed")),
        };

        let rendered = err.to_string();
        assert!(
            rendered.contains("Denied"),
            "denial label must be surfaced: {rendered}"
        );
        assert!(
            rendered.contains("blocked command"),
            "reason must be preserved: {rendered}"
        );
        Ok(())
    }

    #[test]
    fn enforce_policy_decision_requires_approval_with_error() -> Result<()> {
        let err = match enforce_policy_decision(&PolicyDecision::RequireApproval {
            reason: "needs explicit approval".into(),
        }) {
            Err(error) => error,
            Ok(()) => return Err(anyhow::anyhow!("approval policy unexpectedly allowed")),
        };

        let rendered = err.to_string();
        assert!(
            rendered.contains("Requires approval"),
            "approval label must be surfaced: {rendered}"
        );
        assert!(
            rendered.contains("needs explicit approval"),
            "reason must be preserved: {rendered}"
        );
        Ok(())
    }
}
