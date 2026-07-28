use super::harness::{SpyHarnessAdapter, test_adapters, test_governance};
use crate::effect_result::EffectFailure;
use crate::test_support::*;
use maestria_domain::{
    DomainInput, HarnessRunCompleted, HarnessRunId, KernelState, MaestriaEffect,
};
use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{RwLock, mpsc};

// ── harness failure and terminalization regressions ────────────────────

#[tokio::test]
async fn query_harness_infrastructure_claim_failure_returns_false()
-> Result<(), Box<dyn std::error::Error>> {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));
    let inner_journal = Arc::new(InMemoryEffectJournal::default());
    let journal = Arc::new(FailingClaimFeedbackJournal {
        inner: inner_journal.clone(),
    });

    let adapters = Arc::new(Adapters {
        harness,
        effect_journal: journal,
        ..crate::test_helpers::test_adapters()
    });
    let governance = test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let run_id = maestria_domain::HarnessRunId(100);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo test".to_string(),
    };

    let ctx = EffectExecutionContext::test_default(
        adapters.clone(),
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(
        !result,
        "infrastructure claim failure must return false, not success"
    );
    assert!(
        harness_called.load(Ordering::Relaxed),
        "harness must have been invoked"
    );
    let in_flight = adapters.effect_journal.scan_in_flight()?;
    assert_eq!(
        in_flight.len(),
        1,
        "journal entry must remain in-flight after claim failure"
    );
    assert_eq!(in_flight[0].run_id, run_id);
    assert!(
        adapters
            .effect_journal
            .is_current(run_id, in_flight[0].generation)?,
        "entry must still be current"
    );
    Ok(())
}

#[tokio::test]
async fn query_harness_record_terminal_failure_observable() -> Result<(), Box<dyn std::error::Error>>
{
    let harness = Arc::new(FailingHarnessAdapter);
    let inner_journal = Arc::new(InMemoryEffectJournal::default());
    let journal = Arc::new(FailingRecordTerminalJournal {
        inner: inner_journal.clone(),
    });

    let adapters = Arc::new(Adapters {
        harness,
        effect_journal: journal,
        ..crate::test_helpers::test_adapters()
    });
    let governance = test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let run_id = maestria_domain::HarnessRunId(101);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo test".to_string(),
    };

    let ctx = EffectExecutionContext::test_default(
        adapters.clone(),
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result =
        MaestriaRuntime::test_execute_effect(MaestriaEffect::QueryHarness(request), ctx, None)
            .await;

    assert!(
        !result,
        "harness execution error with terminalization failure must return false"
    );
    let in_flight = adapters.effect_journal.scan_in_flight()?;
    assert_eq!(
        in_flight.len(),
        1,
        "journal entry must remain in-flight when record_terminal fails"
    );
    assert_eq!(in_flight[0].run_id, run_id);
    assert!(
        adapters
            .effect_journal
            .is_current(run_id, in_flight[0].generation)?,
        "entry must still be current after failed terminalization"
    );
    Ok(())
}

#[tokio::test]
async fn query_harness_scope_denial_preserves_typed_reason()
-> Result<(), Box<dyn std::error::Error>> {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));
    let adapters = test_adapters(harness);
    let governance = test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let request = maestria_domain::QueryHarnessRequest {
        run_id: HarnessRunId(102),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo denied".to_string(),
    };
    let ctx = EffectExecutionContext {
        scope: Scope::new(vec![], vec![], vec![], vec![], false),
        ..EffectExecutionContext::test_default(
            adapters,
            governance,
            Arc::new(RwLock::new(KernelState::new())),
            input_tx,
        )
    };

    let result = ctx
        .execute_effect(MaestriaEffect::QueryHarness(request), None)
        .await;
    match result {
        Err(EffectFailure::Denied(reason)) => {
            assert!(
                reason.contains("not allowed by scope"),
                "denial reason should identify scope rejection: {reason}"
            );
        }
        Err(other) => return Err(format!("expected typed denial, got {other}").into()),
        Ok(()) => return Err("scope-denied harness unexpectedly succeeded".into()),
    }
    assert!(
        !harness_called.load(Ordering::Relaxed),
        "scope-denied harness must not be invoked"
    );
    Ok(())
}

#[tokio::test]
async fn query_harness_full_input_channel_pauses_and_fails_effect()
-> Result<(), Box<dyn std::error::Error>> {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));
    let adapters = test_adapters(harness);
    let governance = test_governance();
    let (input_tx, mut input_rx) = mpsc::channel(1);
    input_tx
        .try_send(DomainInput::HarnessRunCompleted(HarnessRunCompleted {
            run_id: HarnessRunId(999),
            generation: 1,
            task_id: None,
            command: "occupied".to_string(),
            exit_code: 0,
            output: String::new(),
        }))
        .map_err(|error| format!("failed to fill input channel: {error}"))?;

    let run_id = HarnessRunId(103);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo full".to_string(),
    };
    let ctx = EffectExecutionContext::test_default(
        adapters.clone(),
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );

    let result = ctx
        .execute_effect(MaestriaEffect::QueryHarness(request), None)
        .await;
    match result {
        Err(EffectFailure::Degraded(reason)) => {
            assert!(
                reason.contains("completion delivery failed"),
                "delivery failure reason should be preserved: {reason}"
            );
            assert!(reason.contains("effect paused"));
        }
        Err(other) => return Err(format!("expected paused degraded failure, got {other}").into()),
        Ok(()) => return Err("full input channel unexpectedly succeeded".into()),
    }
    let is_current = adapters.effect_journal.is_current(run_id, 1)?;
    assert!(
        !is_current,
        "paused harness generation must not remain current"
    );
    let feedback_accepted = adapters.effect_journal.is_feedback_accepted(run_id, 1)?;
    assert!(
        !feedback_accepted,
        "paused harness generation must not remain feedback-accepted"
    );
    assert!(matches!(
        input_rx.try_recv(),
        Ok(DomainInput::HarnessRunCompleted(_))
    ));
    Ok(())
}

#[tokio::test]
async fn query_harness_closed_input_channel_pauses_and_fails_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));
    let adapters = test_adapters(harness);
    let governance = test_governance();
    let (input_tx, input_rx) = mpsc::channel(8);
    drop(input_rx);

    let run_id = HarnessRunId(104);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo closed".to_string(),
    };
    let ctx = EffectExecutionContext::test_default(
        adapters.clone(),
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );

    let result = ctx
        .execute_with_retries(MaestriaEffect::QueryHarness(request))
        .await;
    match result {
        Err(EffectFailure::Degraded(reason)) => {
            assert!(reason.contains("completion delivery failed"));
            assert!(reason.contains("effect paused"));
        }
        Err(other) => return Err(format!("expected closed-channel failure, got {other}").into()),
        Ok(()) => return Err("closed input channel unexpectedly succeeded".into()),
    }
    let is_current = adapters.effect_journal.is_current(run_id, 1)?;
    assert!(!is_current);
    Ok(())
}

#[tokio::test]
async fn query_harness_pause_failure_remains_observable() -> Result<(), Box<dyn std::error::Error>>
{
    let harness_called = Arc::new(AtomicBool::new(false));
    let harness = Arc::new(SpyHarnessAdapter::new(harness_called.clone()));
    let inner_journal = Arc::new(InMemoryEffectJournal::default());
    let journal = Arc::new(FailingPauseJournal {
        inner: inner_journal,
    });
    let adapters = Arc::new(Adapters {
        harness,
        effect_journal: journal,
        ..crate::test_helpers::test_adapters()
    });
    let governance = test_governance();
    let (input_tx, input_rx) = mpsc::channel(8);
    drop(input_rx);

    let run_id = HarnessRunId(105);
    let request = maestria_domain::QueryHarnessRequest {
        run_id,
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
        approval_id: None,
        command: "echo pause-failure".to_string(),
    };
    let ctx = EffectExecutionContext::test_default(
        adapters.clone(),
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );

    let result = ctx
        .execute_with_retries(MaestriaEffect::QueryHarness(request))
        .await;
    match result {
        Err(EffectFailure::Failed(reason)) => {
            assert!(reason.contains("completion delivery failed"));
            assert!(reason.contains("failed to pause harness effect"));
        }
        Err(other) => return Err(format!("expected pause failure, got {other}").into()),
        Ok(()) => return Err("pause journal failure unexpectedly succeeded".into()),
    }
    let feedback_accepted = adapters.effect_journal.is_feedback_accepted(run_id, 1)?;
    assert!(
        feedback_accepted,
        "failed pause must leave feedback acceptance observable"
    );
    Ok(())
}

// ── failure-preservation test helpers ──────────────────────────────────

struct FailingHarnessAdapter;

impl HarnessAdapter for FailingHarnessAdapter {
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
        _request: HarnessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HarnessOutcome, PortError>> + Send + '_>> {
        Box::pin(async move {
            Err(PortError::Downstream {
                message: "simulated harness failure".to_string(),
            })
        })
    }
}

struct FailingClaimFeedbackJournal {
    inner: Arc<dyn EffectJournal + Send + Sync>,
}

impl EffectJournal for FailingClaimFeedbackJournal {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        self.inner.record_intent(intent)
    }
    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }
    fn claim_feedback(&self, _run_id: HarnessRunId, _generation: u64) -> Result<(), PortError> {
        Err(PortError::Internal {
            message: "simulated claim_feedback failure".to_string(),
        })
    }
    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        self.inner.record_terminal(run_id, generation, status)
    }
    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        self.inner.scan_in_flight()
    }
    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        self.inner.is_current(run_id, generation)
    }
}

struct FailingRecordTerminalJournal {
    inner: Arc<dyn EffectJournal + Send + Sync>,
}

impl EffectJournal for FailingRecordTerminalJournal {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        self.inner.record_intent(intent)
    }
    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }
    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.claim_feedback(run_id, generation)
    }
    fn record_terminal(
        &self,
        _run_id: HarnessRunId,
        _generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        if status == EffectJournalStatus::Failed {
            Err(PortError::Internal {
                message: "simulated record_terminal(Failed) failure".to_string(),
            })
        } else {
            Ok(())
        }
    }
    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        self.inner.scan_in_flight()
    }
    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        self.inner.is_current(run_id, generation)
    }
}

struct FailingPauseJournal {
    inner: Arc<dyn EffectJournal + Send + Sync>,
}

impl EffectJournal for FailingPauseJournal {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        self.inner.record_intent(intent)
    }
    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }
    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.claim_feedback(run_id, generation)
    }
    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        if status == EffectJournalStatus::Paused {
            Err(PortError::Internal {
                message: "simulated record_terminal(Paused) failure".to_string(),
            })
        } else {
            self.inner.record_terminal(run_id, generation, status)
        }
    }
    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        self.inner.scan_in_flight()
    }
    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        self.inner.is_current(run_id, generation)
    }
}
