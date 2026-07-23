use crate::test_support::*;
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

// ── harness governance integration tests ───────────────────────────────

/// Verify that a QueryHarness effect with an invalid command (non-grammar)
/// returns true (no error) but never invokes the harness adapter.
#[tokio::test]
async fn query_harness_denies_invalid_grammar_before_spawn()
-> Result<(), Box<dyn std::error::Error>> {
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

    assert!(!result, "denied commands must report a failed effect");
    assert!(
        !harness_called.load(Ordering::Relaxed),
        "harness must not be invoked for denied commands"
    );
    Ok(())
}

/// Verify that a cat command targeting a path outside readable roots
/// is rejected before spawning.
#[tokio::test]
async fn query_harness_rejects_cat_outside_scope() -> Result<(), Box<dyn std::error::Error>> {
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

    assert!(result, "rejected command outcome should be recorded");
    assert!(
        !harness_called.load(Ordering::Relaxed),
        "harness must not be invoked for out-of-scope cat"
    );
    Ok(())
}

/// Verify that an allowed command (echo) proceeds through to the adapter
/// and produces a HarnessRunCompleted event.
#[tokio::test]
async fn query_harness_allows_grammar_compliant_echo() -> Result<(), Box<dyn std::error::Error>> {
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
        .await?
        .ok_or("HarnessRunCompleted should be sent")?;

    assert!(
        matches!(completed, DomainInput::HarnessRunCompleted { .. }),
        "expected HarnessRunCompleted, got {:?}",
        std::mem::discriminant(&completed)
    );
    Ok(())
}

/// Verify that pwd command proceeds to the adapter and completion event fires.
#[tokio::test]
async fn query_harness_allows_pwd() -> Result<(), Box<dyn std::error::Error>> {
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
        .await?
        .ok_or("HarnessRunCompleted should be sent")?;

    assert!(matches!(completed, DomainInput::HarnessRunCompleted { .. }));
    Ok(())
}

/// Verify that cat with a path inside readable roots succeeds.
#[tokio::test]
async fn query_harness_allows_cat_within_scope() -> Result<(), Box<dyn std::error::Error>> {
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

    let completed = match tokio::time::timeout(Duration::from_millis(500), input_rx.recv()).await {
        Ok(Some(input)) => input,
        Ok(None) => return Err("harness completion channel closed".into()),
        Err(_) => return Err("timed out waiting for harness completion".into()),
    };

    assert!(matches!(completed, DomainInput::HarnessRunCompleted { .. }));
    Ok(())
}

#[tokio::test]
async fn query_harness_records_lifecycle_and_processes_current_feedback()
-> Result<(), Box<dyn std::error::Error>> {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let run_id = maestria_domain::HarnessRunId(10);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "pwd".to_string(),
    };

    let ctx = EffectExecutionContext {
        scope: Scope::new(vec![], vec![], vec!["shell".into()], vec![], false),
        ..EffectExecutionContext::test_default(
            adapters.clone(),
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };

    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(result);

    let completed = match tokio::time::timeout(Duration::from_millis(500), input_rx.recv()).await {
        Ok(Some(input)) => input,
        Ok(None) => return Err("harness completion channel closed".into()),
        Err(_) => return Err("timed out waiting for harness completion".into()),
    };

    let (completed_run_id, completed_generation) = match &completed {
        DomainInput::HarnessRunCompleted(input) => (input.run_id, input.generation),
        _ => return Err("expected HarnessRunCompleted input".into()),
    };
    adapters.effect_journal.record_terminal(
        completed_run_id,
        completed_generation,
        maestria_ports::EffectJournalStatus::Completed,
    )?;
    assert!(
        adapters.effect_journal.scan_in_flight()?.is_empty(),
        "acknowledged feedback should not remain in flight"
    );

    Ok(())
}

#[tokio::test]
async fn query_harness_rejects_stale_feedback_when_not_current()
-> Result<(), Box<dyn std::error::Error>> {
    struct DelayingHarnessAdapter {
        rx: tokio::sync::Mutex<tokio::sync::oneshot::Receiver<()>>,
    }
    impl HarnessAdapter for DelayingHarnessAdapter {
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
            Box::pin(async move {
                let mut rx = self.rx.lock().await;
                let _ = (&mut *rx).await;
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

    let (tx, rx) = tokio::sync::oneshot::channel();
    let harness = Arc::new(DelayingHarnessAdapter {
        rx: tokio::sync::Mutex::new(rx),
    });

    let adapters = test_adapters(harness.clone());
    let governance = test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let run_id = maestria_domain::HarnessRunId(42);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo test".to_string(),
    };

    let ctx = EffectExecutionContext {
        scope: Scope::new(vec![], vec![], vec!["shell".into()], vec![], false),
        ..EffectExecutionContext::test_default(
            adapters.clone(),
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };

    let effect_task = tokio::spawn(async move {
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None).await
    });

    // Give the effect handler time to record intent and start executing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Manually supersede the entry to make the current execution stale
    let intent = maestria_ports::EffectJournalIntent {
        run_id,
        task_id: None,
        capability: "shell".to_string(),
        command: "echo test".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        requested_generation: None,
    };
    let _ = adapters.effect_journal.record_intent(intent)?;

    // Unblock the execution
    let _ = tx.send(());

    let result = effect_task.await?;
    assert!(
        result,
        "stale execution should still return true to avoid retry loops"
    );

    let msg = tokio::time::timeout(Duration::from_millis(50), input_rx.recv()).await;
    assert!(
        msg.is_err() || matches!(msg, Ok(None)),
        "should not have received message, but got: {:?}",
        msg
    );
    Ok(())
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
