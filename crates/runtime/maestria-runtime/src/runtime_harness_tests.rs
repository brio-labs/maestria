use super::shell_policy::{cat_path_args, is_shell_grammar_allowed, resolve_working_directory};
use super::test_support::*;
use maestria_domain::{DomainInput, KernelState, MaestriaEffect};
use maestria_governance::Scope;
use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
// ── harness grammar tests ──────────────────────────────────────────────

#[test]
fn grammar_allows_echo_pwd_cat() {
    assert!(is_shell_grammar_allowed("echo hello world"));
    assert!(is_shell_grammar_allowed("echo"));
    assert!(is_shell_grammar_allowed("pwd"));
    assert!(is_shell_grammar_allowed("cat /tmp/file.txt"));
    assert!(is_shell_grammar_allowed("cat file1.txt file2.txt"));
    assert!(is_shell_grammar_allowed("  echo  spaced  "));
}

#[test]
fn grammar_rejects_unknown_commands() {
    assert!(!is_shell_grammar_allowed("ls"));
    assert!(!is_shell_grammar_allowed("rm -rf /"));
    assert!(!is_shell_grammar_allowed("curl example.com"));
    assert!(!is_shell_grammar_allowed("bash"));
    assert!(!is_shell_grammar_allowed("sh"));
}

#[test]
fn grammar_rejects_metacharacters() {
    assert!(!is_shell_grammar_allowed("echo hello | cat"));
    assert!(!is_shell_grammar_allowed("echo hello && pwd"));
    assert!(!is_shell_grammar_allowed("echo $HOME"));
    assert!(!is_shell_grammar_allowed("echo `whoami`"));
    assert!(!is_shell_grammar_allowed("echo $(whoami)"));
    assert!(!is_shell_grammar_allowed("cat file > /dev/null"));
    assert!(!is_shell_grammar_allowed("cat < /etc/passwd"));
    assert!(!is_shell_grammar_allowed("echo hello ; rm -rf /"));
    assert!(!is_shell_grammar_allowed("cat /tmp/*"));
    assert!(!is_shell_grammar_allowed("echo ~/file"));
    assert!(!is_shell_grammar_allowed("echo hello &"));
    assert!(!is_shell_grammar_allowed("echo hello\ncat /etc/passwd"));
    assert!(!is_shell_grammar_allowed("echo hello\\nworld"));
}

#[test]
fn cat_path_args_extracts_paths() {
    let args = cat_path_args("cat /tmp/a.txt /tmp/b.txt");
    assert_eq!(args, vec!["/tmp/a.txt", "/tmp/b.txt"]);

    let args = cat_path_args("cat single.txt");
    assert_eq!(args, vec!["single.txt"]);

    let args = cat_path_args("echo hello");
    assert!(args.is_empty());

    let args = cat_path_args("pwd");
    assert!(args.is_empty());
}

#[test]
fn resolve_working_directory_returns_first_read_root() {
    let scope = Scope::new(
        vec![PathBuf::from("/workspace")],
        vec![],
        vec![],
        vec![],
        false,
    );
    let wd = resolve_working_directory(&scope).expect("read root should resolve");
    assert_eq!(wd, PathBuf::from("/workspace"));
}

#[test]
fn resolve_working_directory_falls_back_when_no_roots() {
    let scope = Scope::default();
    let wd = resolve_working_directory(&scope).expect("current directory should resolve");
    assert!(!wd.as_os_str().is_empty());
}

// ── harness governance integration tests ───────────────────────────────

/// Verify that a QueryHarness effect with an invalid command (non-grammar)
/// returns true (no error) but never invokes the harness adapter.
#[tokio::test]
async fn query_harness_denies_invalid_grammar_before_spawn() {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let request = maestria_domain::QueryHarnessRequest {
        run_id: maestria_domain::HarnessRunId(1),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "rm -rf /".to_string(),
    };

    let ctx = EffectExecutionContext {
        scope: Scope::new(
            vec![PathBuf::from("/workspace")],
            vec![],
            vec!["shell".into()],
            vec![],
            false,
        ),
        ..EffectExecutionContext::test_default(
            adapters,
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(result, "denied commands should return true (non-fatal)");
    assert!(
        !harness_called.load(Ordering::Relaxed),
        "harness must not be invoked for denied commands"
    );
}

/// Verify that a cat command targeting a path outside readable roots
/// is rejected before spawning.
#[tokio::test]
async fn query_harness_rejects_cat_outside_scope() {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let request = maestria_domain::QueryHarnessRequest {
        run_id: maestria_domain::HarnessRunId(2),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "cat /etc/passwd".to_string(),
    };

    let ctx = EffectExecutionContext {
        scope: Scope::new(
            vec![PathBuf::from("/workspace")],
            vec![],
            vec!["shell".into()],
            vec![],
            false,
        ),
        ..EffectExecutionContext::test_default(
            adapters,
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(result, "out-of-scope cat should return true (non-fatal)");
    assert!(
        !harness_called.load(Ordering::Relaxed),
        "harness must not be invoked for out-of-scope cat"
    );
}

/// Verify that an allowed command (echo) proceeds through to the adapter
/// and produces a HarnessRunCompleted event.
#[tokio::test]
async fn query_harness_allows_grammar_compliant_echo() {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let request = maestria_domain::QueryHarnessRequest {
        run_id: maestria_domain::HarnessRunId(3),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo hello world".to_string(),
    };

    let ctx = EffectExecutionContext {
        scope: Scope::new(
            vec![PathBuf::from("/workspace")],
            vec![],
            vec!["shell".into()],
            vec![],
            false,
        ),
        ..EffectExecutionContext::test_default(
            adapters,
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(result, "allowed command should succeed");
    assert!(
        harness_called.load(Ordering::Relaxed),
        "harness must be invoked for allowed commands"
    );

    // Verify HarnessRunCompleted was sent
    let completed = tokio::time::timeout(Duration::from_millis(500), input_rx.recv())
        .await
        .expect("timed out waiting for HarnessRunCompleted")
        .expect("HarnessRunCompleted should be sent");

    assert!(
        matches!(completed, DomainInput::HarnessRunCompleted { .. }),
        "expected HarnessRunCompleted, got {:?}",
        std::mem::discriminant(&completed)
    );
}

/// Verify that pwd command proceeds to the adapter and completion event fires.
#[tokio::test]
async fn query_harness_allows_pwd() {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let request = maestria_domain::QueryHarnessRequest {
        run_id: maestria_domain::HarnessRunId(4),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "pwd".to_string(),
    };

    let ctx = EffectExecutionContext::test_default(
        adapters,
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(result);
    assert!(harness_called.load(Ordering::Relaxed));

    let completed = tokio::time::timeout(Duration::from_millis(500), input_rx.recv())
        .await
        .expect("timed out")
        .expect("HarnessRunCompleted should be sent");

    assert!(matches!(completed, DomainInput::HarnessRunCompleted { .. }));
}

/// Verify that cat with a path inside readable roots succeeds.
#[tokio::test]
async fn query_harness_allows_cat_within_scope() {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let request = maestria_domain::QueryHarnessRequest {
        run_id: maestria_domain::HarnessRunId(5),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "cat /workspace/file.txt".to_string(),
    };

    let ctx = EffectExecutionContext {
        scope: Scope::new(
            vec![PathBuf::from("/workspace")],
            vec![],
            vec!["shell".into()],
            vec![],
            false,
        ),
        ..EffectExecutionContext::test_default(
            adapters,
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(result);
    assert!(
        harness_called.load(Ordering::Relaxed),
        "harness must be invoked for in-scope cat"
    );

    let completed = tokio::time::timeout(Duration::from_millis(500), input_rx.recv())
        .await
        .expect("timed out")
        .expect("HarnessRunCompleted should be sent");

    assert!(matches!(completed, DomainInput::HarnessRunCompleted { .. }));
}

// ── spy harness adapter for integration tests ──────────────────────────

struct SpyHarnessAdapter {
    called: Arc<AtomicBool>,
}

impl SpyHarnessAdapter {
    fn new(called: Arc<AtomicBool>) -> Self {
        Self { called }
    }
}

impl HarnessAdapter for SpyHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(HarnessCapabilities {
            command_classes: vec![HarnessCommandClass::Shell],
            write_enabled: true,
            read_enabled: true,
            web_enabled: false,
        })
    }
    fn execute(
        &self,
        request: HarnessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HarnessOutcome, PortError>> + Send + '_>> {
        let called = self.called.clone();
        Box::pin(async move {
            called.store(true, Ordering::Relaxed);
            Ok(HarnessOutcome {
                run_id: request.run_id,
                command: request.command,
                exit_code: 0,
                stdout: b"output".to_vec(),
                stderr: Vec::new(),
                duration: Duration::from_millis(1),
                artifacts_created: Vec::new(),
                diff_summary: None,
                validation_hints: Vec::new(),
            })
        })
    }
}

fn test_adapters(harness: Arc<dyn HarnessAdapter + Send + Sync>) -> Arc<Adapters> {
    Arc::new(Adapters {
        harness,
        ..crate::test_helpers::test_adapters()
    })
}

fn test_governance() -> Arc<Governance> {
    Arc::new(crate::test_helpers::test_governance())
}
