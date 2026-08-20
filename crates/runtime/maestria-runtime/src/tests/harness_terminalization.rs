use super::harness::{SpyHarnessAdapter, test_adapters, test_governance};
use crate::effect_result::EffectFailure;
use crate::test_support::*;
use maestria_domain::{
    DomainInput, HarnessRunCompleted, HarnessRunId, KernelState, MaestriaEffect,
    ModelAgentProposalExecution, ModelAgentProposalRequest,
};
use maestria_ports::{
    EffectJournal, EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, HarnessAdapter,
    HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest, PortError,
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
        execution: maestria_domain::HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
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
        execution: maestria_domain::HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
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
        execution: maestria_domain::HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
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
        execution: maestria_domain::HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
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
    let is_current = adapters
        .effect_journal
        .is_current(run_id, maestria_domain::JournalGeneration::new(1))?;
    assert!(
        !is_current,
        "paused harness generation must not remain current"
    );
    let feedback_accepted = adapters
        .effect_journal
        .is_feedback_accepted(run_id, maestria_domain::JournalGeneration::new(1))?;
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
        execution: maestria_domain::HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
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
    let is_current = adapters
        .effect_journal
        .is_current(run_id, maestria_domain::JournalGeneration::new(1))?;
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
        execution: maestria_domain::HarnessExecution::Fresh,
        capability: "shell".to_string(),
        scope_id: maestria_domain::ScopeId(1),
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
    let feedback_accepted = adapters
        .effect_journal
        .is_feedback_accepted(run_id, maestria_domain::JournalGeneration::new(1))?;
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
    fn record_started(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }
    fn claim_feedback(
        &self,
        _run_id: HarnessRunId,
        _generation: maestria_domain::JournalGeneration,
    ) -> Result<(), PortError> {
        Err(PortError::internal(
            "maestria runtime test",
            "simulated claim_feedback failure".to_string(),
        ))
    }
    fn claim_feedback_with_outcome(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
        _outcome: HarnessOutcome,
    ) -> Result<(), PortError> {
        self.claim_feedback(run_id, generation)
    }

    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
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
        generation: maestria_domain::JournalGeneration,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }
    fn is_current(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<bool, PortError> {
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
    fn record_started(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }
    fn claim_feedback(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<(), PortError> {
        self.inner.claim_feedback(run_id, generation)
    }
    fn claim_feedback_with_outcome(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
        _outcome: HarnessOutcome,
    ) -> Result<(), PortError> {
        self.claim_feedback(run_id, generation)
    }

    fn record_terminal(
        &self,
        _run_id: HarnessRunId,
        _generation: maestria_domain::JournalGeneration,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        if status == EffectJournalStatus::Failed {
            Err(PortError::internal(
                "maestria runtime test",
                "simulated record_terminal(Failed) failure".to_string(),
            ))
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
        generation: maestria_domain::JournalGeneration,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }
    fn is_current(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<bool, PortError> {
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
    fn record_started(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }
    fn claim_feedback(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<(), PortError> {
        self.inner.claim_feedback(run_id, generation)
    }
    fn claim_feedback_with_outcome(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
        _outcome: HarnessOutcome,
    ) -> Result<(), PortError> {
        self.claim_feedback(run_id, generation)
    }

    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        if status == EffectJournalStatus::Paused {
            Err(PortError::internal(
                "maestria runtime test",
                "simulated record_terminal(Paused) failure".to_string(),
            ))
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
        generation: maestria_domain::JournalGeneration,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }
    fn is_current(
        &self,
        run_id: HarnessRunId,
        generation: maestria_domain::JournalGeneration,
    ) -> Result<bool, PortError> {
        self.inner.is_current(run_id, generation)
    }
}

#[tokio::test]
async fn model_agent_recovery_consumes_stored_success_without_reexecution()
-> Result<(), Box<dyn std::error::Error>> {
    let called = Arc::new(AtomicBool::new(false));
    let adapters = test_adapters(Arc::new(SpyHarnessAdapter::new(called.clone())));
    let run_id = HarnessRunId::new(700);
    let entry = adapters.effect_journal.record_intent(EffectJournalIntent {
        run_id,
        task_id: None,
        capability: "shell".to_string(),
        command: "echo recovered".to_string(),
        scope_id: maestria_domain::ScopeId::new(1),
        requested_generation: None,
    })?;
    adapters
        .effect_journal
        .record_started(run_id, entry.generation)?;
    let outcome = HarnessOutcome {
        run_id,
        command: "echo recovered".to_string(),
        exit_code: 0,
        stdout: b"recovered".to_vec(),
        stderr: Vec::new(),
        duration: std::time::Duration::from_millis(1),
        artifacts_created: Vec::new(),
        diff_summary: None,
        validation_hints: Vec::new(),
    };
    adapters
        .effect_journal
        .claim_feedback_with_outcome(run_id, entry.generation, outcome)?;
    let canonical = ModelAgentProposalRequest {
        run_id,
        task_id: None,
        query: "recovery search must not run".to_string(),
        limit: 1,
        evidence_ids: Vec::new(),
        capability: "shell".to_string(),
        command: "echo recovered".to_string(),
        working_directory: String::new(),
        timeout_secs: 1,
        expected_generation: maestria_domain::IndexGenerationId::new(1),
        task_validation: false,
        memory_candidate: false,
        execution: ModelAgentProposalExecution::Fresh,
        correlation_id: maestria_domain::CorrelationId::new(9),
    };
    let mut recovery = canonical.clone();
    recovery.execution = ModelAgentProposalExecution::JournalRecovery {
        journal_generation: entry.generation,
    };
    let mut state = KernelState::new();
    state.model_agent_requests.insert(run_id, canonical);
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let context = EffectExecutionContext::test_default(
        adapters.clone(),
        test_governance(),
        Arc::new(RwLock::new(state)),
        input_tx,
    );
    let result = context
        .execute_effect(
            MaestriaEffect::QueryHarnessProposal(Box::new(recovery)),
            None,
        )
        .await;
    result.map_err(|error| format!("recovery proposal failed: {error}"))?;
    assert!(
        !called.load(Ordering::Relaxed),
        "recovery must not rerun harness"
    );
    assert!(matches!(
        input_rx.recv().await,
        Some(DomainInput::HarnessRunCompleted(HarnessRunCompleted {
            run_id: recovered_run,
            exit_code: 0,
            ..
        })) if recovered_run == run_id
    ));
    assert!(matches!(
        input_rx.recv().await,
        Some(DomainInput::ModelAgentProposalCompleted(result))
            if result.run_id() == run_id && !result.is_failed()
    ));
    let in_flight = adapters.effect_journal.scan_in_flight()?;
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].generation, entry.generation);
    assert_eq!(in_flight[0].status, EffectJournalStatus::FeedbackAccepted);
    adapters.effect_journal.record_terminal(
        run_id,
        entry.generation,
        EffectJournalStatus::Completed,
    )?;
    assert!(adapters.effect_journal.scan_in_flight()?.is_empty());
    Ok(())
}

fn journal_recovery_proposal(run_id: HarnessRunId) -> ModelAgentProposalRequest {
    ModelAgentProposalRequest {
        run_id,
        task_id: None,
        query: "recovery search must not run".to_string(),
        limit: 1,
        evidence_ids: Vec::new(),
        capability: "shell".to_string(),
        command: "echo recovered".to_string(),
        working_directory: String::new(),
        timeout_secs: 1,
        expected_generation: maestria_domain::IndexGenerationId::new(1),
        task_validation: false,
        memory_candidate: false,
        execution: ModelAgentProposalExecution::Fresh,
        correlation_id: maestria_domain::CorrelationId::new(10),
    }
}

#[tokio::test]
async fn journal_recovery_rejects_missing_or_invalid_durable_feedback()
-> Result<(), Box<dyn std::error::Error>> {
    for case in [
        "missing",
        "feedback_without_outcome",
        "mismatched_record",
        "mismatched_outcome",
    ] {
        let called = Arc::new(AtomicBool::new(false));
        let adapters = test_adapters(Arc::new(SpyHarnessAdapter::new(called.clone())));
        let run_id = HarnessRunId::new(710);
        let generation = maestria_domain::JournalGeneration::new(2);
        let canonical = journal_recovery_proposal(run_id);
        if case != "missing" {
            let entry = adapters.effect_journal.record_intent(EffectJournalIntent {
                run_id,
                task_id: None,
                capability: "shell".to_string(),
                command: if case == "mismatched_record" {
                    "echo different".to_string()
                } else {
                    canonical.command.clone()
                },
                scope_id: maestria_domain::ScopeId::new(1),
                requested_generation: Some(generation),
            })?;
            assert_eq!(entry.generation, generation);
            if case == "feedback_without_outcome" {
                adapters.effect_journal.record_started(run_id, generation)?;
                adapters.effect_journal.claim_feedback(run_id, generation)?;
            } else if case == "mismatched_outcome" {
                adapters.effect_journal.record_started(run_id, generation)?;
                adapters.effect_journal.claim_feedback_with_outcome(
                    run_id,
                    generation,
                    HarnessOutcome {
                        run_id: HarnessRunId::new(999),
                        command: "echo other".to_string(),
                        exit_code: 0,
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                        duration: std::time::Duration::from_millis(1),
                        artifacts_created: Vec::new(),
                        diff_summary: None,
                        validation_hints: Vec::new(),
                    },
                )?;
            }
        }
        let mut recovery = canonical.clone();
        recovery.execution = ModelAgentProposalExecution::JournalRecovery {
            journal_generation: generation,
        };
        let mut state = KernelState::new();
        state.model_agent_requests.insert(run_id, canonical);
        let (input_tx, mut input_rx) = mpsc::channel(8);
        let context = EffectExecutionContext::test_default(
            adapters.clone(),
            test_governance(),
            Arc::new(RwLock::new(state)),
            input_tx,
        );
        let result = context
            .execute_effect(
                MaestriaEffect::QueryHarnessProposal(Box::new(recovery)),
                None,
            )
            .await;
        if case == "mismatched_outcome" {
            assert!(
                result.is_ok(),
                "{case} recovery returned the wrong handler result: {result:?}"
            );
            assert!(matches!(
                input_rx.try_recv(),
                Ok(DomainInput::ModelAgentProposalCompleted(result)) if result.is_failed()
            ));
            assert!(
                input_rx.try_recv().is_err(),
                "{case} recovery emitted provider completion"
            );
        } else {
            assert!(
                matches!(result, Err(EffectFailure::Denied(_))),
                "{case} recovery unexpectedly admitted: {result:?}"
            );
            assert!(
                input_rx.try_recv().is_err(),
                "{case} recovery emitted a terminal result"
            );
        }
        assert!(
            !called.load(Ordering::Relaxed),
            "{case} recovery invoked harness provider"
        );
    }
    Ok(())
}
