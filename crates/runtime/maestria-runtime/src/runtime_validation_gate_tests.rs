use crate::test_helpers;
use crate::{Adapters, Governance, MaestriaRuntime, RuntimeConfig};
use maestria_domain::{
    CompleteTaskInput, DomainEvent, DomainInput, KernelState, Task, TaskId, TaskStatus,
    ValidationReportId, ValidationReportRecord,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{EventFilter, EventLog, InMemoryEventLog};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn run_complete_task_test(
    state: KernelState,
    governance: Governance,
    task_id: TaskId,
    report_id: ValidationReportId,
) -> Result<Vec<maestria_domain::DomainEventEnvelope>, Box<dyn std::error::Error>> {
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

    input_tx
        .send(DomainInput::CompleteTask(CompleteTaskInput {
            task_id,
            validation_report_id: report_id,
        }))
        .await
        .map_err(|e| format!("send failed: {}", e))?;

    tokio::time::sleep(Duration::from_millis(50)).await;

    shutdown_token.cancel();
    let _ = runtime_handle.await;

    let events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_err(|e| format!("scan failed: {:?}", e))?;
    Ok(events)
}

#[tokio::test]
async fn task_completion_blocked_by_missing_report() -> Result<(), Box<dyn std::error::Error>> {
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

    let governance = test_helpers::test_governance();
    let events =
        run_complete_task_test(state, governance, task_id, ValidationReportId::new(99)).await?;

    // No events should be emitted since it was blocked
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
    let report = ValidationReportRecord {
        task_id: Some(task_id),
        passed: false,
        warnings: vec![],
    };
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(report_id, report);

    let governance = test_helpers::test_governance();
    let events = run_complete_task_test(state, governance, task_id, report_id).await?;

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
    let report = ValidationReportRecord {
        task_id: Some(TaskId::new(2)), // mismatch
        passed: true,
        warnings: vec![],
    };
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(report_id, report);

    let governance = test_helpers::test_governance();
    let events = run_complete_task_test(state, governance, task_id, report_id).await?;

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
    let report = ValidationReportRecord {
        task_id: Some(task_id),
        passed: true,
        warnings: vec![],
    };
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(report_id, report);

    let governance = test_helpers::test_governance();
    let events = run_complete_task_test(state, governance, task_id, report_id).await?;

    assert_eq!(events.len(), 1, "expected 1 event, got {:?}", events);
    match &events[0].event {
        DomainEvent::TaskCompletionRecorded { status, .. } => {
            assert_eq!(*status, TaskStatus::CompletedVerified);
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
    let report = ValidationReportRecord {
        task_id: Some(task_id),
        passed: true,
        warnings: vec!["some warning".to_string()],
    };
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(report_id, report);

    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(false)), // DISALLOWED
    };
    let events = run_complete_task_test(state, governance, task_id, report_id).await?;

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
    let report = ValidationReportRecord {
        task_id: Some(task_id),
        passed: true,
        warnings: vec!["some warning".to_string()],
    };
    state.tasks.insert(task.id, task);
    state.validation_reports.insert(report_id, report);

    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)), // ALLOWED
    };
    let events = run_complete_task_test(state, governance, task_id, report_id).await?;

    assert_eq!(events.len(), 1, "expected 1 event, got {:?}", events);
    match &events[0].event {
        DomainEvent::TaskCompletionRecorded { status, .. } => {
            assert_eq!(*status, TaskStatus::CompletedWithWarnings);
        }
        other => return Err(format!("expected TaskCompletionRecorded, got {:?}", other).into()),
    }
    Ok(())
}
