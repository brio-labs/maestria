use crate::test_helpers;
use crate::{Adapters, EffectExecutionContext, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_domain::{
    CompleteTaskInput, DomainEvent, DomainInput, KernelState, Task, TaskId, TaskStatus,
    ValidationReportId, ValidationReportRecord,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{EventFilter, EventLog, InMemoryEventLog};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

async fn run_complete_task_test(
    mut state: KernelState,
    governance: Governance,
    task_id: TaskId,
    report_id: ValidationReportId,
    seed_events: Vec<DomainEvent>,
) -> Result<Vec<maestria_domain::DomainEventEnvelope>, Box<dyn std::error::Error>> {
    let event_log = Arc::new(InMemoryEventLog::new());
    for event in seed_events {
        let envelope = maestria_domain::DomainEventEnvelope {
            id: maestria_domain::EventId::new(state.event_log.len() as u64 + 1),
            sequence: maestria_domain::SequenceNumber::new(state.event_log.len() as u64 + 1),
            event,
        };
        event_log
            .append(envelope.clone())
            .map_err(|e| format!("seed append failed: {:?}", e))?;
        state.event_log.push(envelope);
    }
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..test_helpers::test_adapters()
    };

    let (runtime, input_rx) =
        MaestriaRuntime::new(RuntimeConfig::default(), state, adapters, governance);

    let input_tx = runtime.handle().input_tx;
    let shutdown_token = CancellationToken::new();
    let runtime_handle = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    input_tx
        .send(DomainInput::CompleteTask(CompleteTaskInput {
            task_id,
            validation_report_id: report_id,
        }))
        .await
        .map_err(|e| format!("send failed: {}", e))?;

    let sync_tick = maestria_domain::LogicalTick::new(999);
    input_tx
        .send(DomainInput::ClockTick(sync_tick))
        .await
        .map_err(|e| format!("send tick failed: {}", e))?;

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let all_events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_err(|e| format!("scan failed: {:?}", e))?;
            if all_events
                .iter()
                .any(|e| matches!(e.event, DomainEvent::TickObserved { at } if at == sync_tick))
            {
                return Ok::<(), Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "timeout waiting for deterministic barrier (ClockTick)")??;

    shutdown_token.cancel();
    let _ = runtime_handle.await;

    let all_events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_err(|e| format!("scan failed: {:?}", e))?;
    let new_events = all_events
        .into_iter()
        .filter(|e| matches!(e.event, DomainEvent::TaskCompletionRecorded { .. }))
        .collect();
    Ok(new_events)
}

#[tokio::test]
async fn task_completion_blocked_by_missing_durable_report()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    state.tasks.insert(task.id, task);
    let report_id = ValidationReportId::new(99);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(task_id),
            passed: true,
            warnings: vec![],
        },
    );

    let governance = test_helpers::test_governance();
    let events = run_complete_task_test(state, governance, task_id, report_id, vec![]).await?;

    assert!(events.is_empty(), "expected no events, got {:?}", events);
    Ok(())
}

#[tokio::test]
async fn task_completion_blocked_by_failed_report() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let report_id = ValidationReportId::new(99);
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(task_id),
            passed: false,
            warnings: vec![],
        },
    );

    let governance = test_helpers::test_governance();
    let seed = vec![DomainEvent::ValidationReportCreated {
        report_id,
        task_id: Some(task_id),
        passed: false,
        warnings: vec![],
    }];
    let events = run_complete_task_test(state, governance, task_id, report_id, seed).await?;

    assert!(events.is_empty(), "expected no events, got {:?}", events);
    Ok(())
}

#[tokio::test]
async fn task_completion_blocked_by_mismatched_report() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let report_id = ValidationReportId::new(99);
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(TaskId::new(2)), // mismatch
            passed: true,
            warnings: vec![],
        },
    );

    let governance = test_helpers::test_governance();
    let seed = vec![DomainEvent::ValidationReportCreated {
        report_id,
        task_id: Some(TaskId::new(2)), // mismatch
        passed: true,
        warnings: vec![],
    }];
    let events = run_complete_task_test(state, governance, task_id, report_id, seed).await?;

    assert!(events.is_empty(), "expected no events, got {:?}", events);
    Ok(())
}

#[tokio::test]
async fn task_completion_allowed() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let report_id = ValidationReportId::new(99);
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(task_id),
            passed: true,
            warnings: vec![],
        },
    );

    let governance = test_helpers::test_governance();
    let seed = vec![DomainEvent::ValidationReportCreated {
        report_id,
        task_id: Some(task_id),
        passed: true,
        warnings: vec![],
    }];
    let events = run_complete_task_test(state, governance, task_id, report_id, seed).await?;

    if events.len() != 1 {
        return Err(format!("expected 1 event, got {:?}", events).into());
    }
    match &events[0].event {
        DomainEvent::TaskCompletionRecorded { status, .. } => {
            if *status != TaskStatus::CompletedVerified {
                return Err(format!("expected CompletedVerified, got {:?}", status).into());
            }
        }
        other => return Err(format!("expected TaskCompletionRecorded, got {:?}", other).into()),
    }
    Ok(())
}

#[tokio::test]
async fn task_completion_blocked_by_warnings_when_disallowed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let report_id = ValidationReportId::new(99);
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(task_id),
            passed: true,
            warnings: vec!["some warning".to_string()],
        },
    );

    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(false)), // DISALLOWED
    };
    let seed = vec![DomainEvent::ValidationReportCreated {
        report_id,
        task_id: Some(task_id),
        passed: true,
        warnings: vec!["some warning".to_string()],
    }];
    let events = run_complete_task_test(state, governance, task_id, report_id, seed).await?;

    assert!(events.is_empty(), "expected no events, got {:?}", events);
    Ok(())
}

#[tokio::test]
async fn task_completion_allowed_with_warnings_when_configured()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let report_id = ValidationReportId::new(99);
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(task_id),
            passed: true,
            warnings: vec!["some warning".to_string()],
        },
    );

    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)), // ALLOWED
    };
    let seed = vec![DomainEvent::ValidationReportCreated {
        report_id,
        task_id: Some(task_id),
        passed: true,
        warnings: vec!["some warning".to_string()],
    }];
    let events = run_complete_task_test(state, governance, task_id, report_id, seed).await?;

    if events.len() != 1 {
        return Err(format!("expected 1 event, got {:?}", events).into());
    }
    match &events[0].event {
        DomainEvent::TaskCompletionRecorded { status, .. } => {
            if *status != TaskStatus::CompletedWithWarnings {
                return Err(format!("expected CompletedWithWarnings, got {:?}", status).into());
            }
        }
        other => return Err(format!("expected TaskCompletionRecorded, got {:?}", other).into()),
    }
    Ok(())
}

#[tokio::test]
async fn back_to_back_record_report_and_complete_task_succeeds()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::default();
    let task_id = TaskId::new(1);
    let task = Task {
        id: task_id,
        title: "test".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let report_id = ValidationReportId::new(99);
    state.tasks.insert(task.id, task);
    let governance = test_helpers::test_governance();
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..test_helpers::test_adapters()
    };

    let (runtime, input_rx) =
        MaestriaRuntime::new(RuntimeConfig::default(), state, adapters, governance);

    let input_tx = runtime.handle().input_tx;
    let shutdown_token = CancellationToken::new();
    let runtime_handle = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    // Send RecordValidationReport
    input_tx
        .send(DomainInput::RecordValidationReport(
            maestria_domain::RecordValidationReportInput {
                report_id,
                task_id: Some(task_id),
                passed: true,
                warnings: vec![],
            },
        ))
        .await
        .map_err(|e| format!("send record report failed: {}", e))?;

    // Send CompleteTask IMMEDIATELY (back-to-back)
    input_tx
        .send(DomainInput::CompleteTask(CompleteTaskInput {
            task_id,
            validation_report_id: report_id,
        }))
        .await
        .map_err(|e| format!("send complete task failed: {}", e))?;

    // Send ClockTick to synchronize
    let sync_tick = maestria_domain::LogicalTick::new(999);
    input_tx
        .send(DomainInput::ClockTick(sync_tick))
        .await
        .map_err(|e| format!("send tick failed: {}", e))?;

    let res = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let all_events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_err(|e| format!("scan failed: {:?}", e))?;
            if all_events
                .iter()
                .any(|e| matches!(e.event, DomainEvent::TickObserved { at } if at == sync_tick))
            {
                return Ok::<(), Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    if res.is_err() {
        return Err("timeout waiting for deterministic barrier (ClockTick)".into());
    }

    shutdown_token.cancel();
    let _ = runtime_handle.await;

    let all_events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_err(|e| format!("scan failed: {:?}", e))?;
    let new_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| matches!(e.event, DomainEvent::TaskCompletionRecorded { .. }))
        .collect();

    if new_events.len() != 1 {
        return Err(format!(
            "expected 1 TaskCompletionRecorded event, got {:?}",
            new_events
        )
        .into());
    }
    match &new_events[0].event {
        DomainEvent::TaskCompletionRecorded { status, .. } => {
            if *status != TaskStatus::CompletedVerified {
                return Err(format!("expected CompletedVerified, got {:?}", status).into());
            }
        }
        other => return Err(format!("expected TaskCompletionRecorded, got {:?}", other).into()),
    }
    Ok(())
}

#[tokio::test]
async fn search_validation_failure_records_a_failed_report()
-> Result<(), Box<dyn std::error::Error>> {
    let task_id = TaskId::new(7);
    let report_id = ValidationReportId::new(8);
    let mut state = KernelState::new();
    state.tasks.insert(
        task_id,
        Task {
            id: task_id,
            title: "search validation".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            status: TaskStatus::Validating,
            validation_report_id: None,
            artifact_ids: Default::default(),
            evidence_ids: Default::default(),
        },
    );
    let outcome = maestria_domain::SearchOutcome {
        trace: maestria_domain::SearchTraceId::new(1),
        trace_data: None,
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("fixture".to_string())?,
        index_generation: maestria_domain::IndexGenerationId::new(1),
        status: maestria_domain::SearchStatus::Answerable,
        evidence: Vec::new(),
        coverage: maestria_domain::EvidenceCoverage {
            percent_covered: 0,
            gaps_identified: vec!["required evidence".to_string()],
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: Vec::new(),
        },
        conflicts: Vec::new(),
    };
    state.event_log.push(maestria_domain::DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::SearchKnowledgeCompleted {
            task_id: Some(task_id),
            plan: None,
            outcome,
        },
    });

    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel(8);
    let ctx = EffectExecutionContext::test_default(
        Arc::new(test_helpers::test_adapters()),
        Arc::new(test_helpers::test_governance()),
        Arc::new(RwLock::new(state)),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        maestria_domain::MaestriaEffect::RunValidation(maestria_domain::RunValidationRequest {
            task_id: Some(task_id),
            claim_id: None,
            validation_report_id: report_id,
        }),
        ctx,
        None,
    )
    .await;
    assert!(result);

    match input_rx
        .recv()
        .await
        .ok_or("validation report input missing")?
    {
        DomainInput::RecordValidationReport(input) => {
            assert_eq!(input.report_id, report_id);
            assert_eq!(input.task_id, Some(task_id));
            assert!(!input.passed);
            assert!(input.warnings.is_empty());
        }
        other => return Err(format!("unexpected validation input: {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn completion_rejects_a_forged_passing_search_report()
-> Result<(), Box<dyn std::error::Error>> {
    let task_id = TaskId::new(9);
    let report_id = ValidationReportId::new(10);
    let task = Task {
        id: task_id,
        title: "search completion".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status: TaskStatus::Validating,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    };
    let outcome = maestria_domain::SearchOutcome {
        trace: maestria_domain::SearchTraceId::new(1),
        trace_data: None,
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("fixture".to_string())?,
        index_generation: maestria_domain::IndexGenerationId::new(1),
        status: maestria_domain::SearchStatus::Answerable,
        evidence: Vec::new(),
        coverage: maestria_domain::EvidenceCoverage {
            percent_covered: 0,
            gaps_identified: vec!["required evidence".to_string()],
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: Vec::new(),
        },
        conflicts: Vec::new(),
    };
    let events = vec![
        DomainEvent::SearchKnowledgeCompleted {
            task_id: Some(task_id),
            plan: None,
            outcome,
        },
        DomainEvent::ValidationReportCreated {
            report_id,
            task_id: Some(task_id),
            passed: true,
            warnings: Vec::new(),
        },
    ];
    let mut state = KernelState::new();
    state.tasks.insert(task_id, task);
    state.validation_reports.insert(
        report_id,
        ValidationReportRecord {
            task_id: Some(task_id),
            passed: true,
            warnings: Vec::new(),
        },
    );

    let events = run_complete_task_test(
        state,
        test_helpers::test_governance(),
        task_id,
        report_id,
        events,
    )
    .await?;
    assert!(events.is_empty(), "forged search report was accepted");
    Ok(())
}
