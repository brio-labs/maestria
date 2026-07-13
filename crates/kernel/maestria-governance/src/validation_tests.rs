use crate::validation::*;
use maestria_domain::{Task, TaskId, TaskStatus, ValidationReportRecord};

fn create_task(id: u64, status: TaskStatus) -> Task {
    Task {
        id: TaskId::new(id),
        title: "Test Task".to_string(),
        priority: maestria_domain::TaskPriority::Normal,
        status,
        validation_report_id: None,
        artifact_ids: Default::default(),
        evidence_ids: Default::default(),
    }
}

fn create_report(task_id: u64, passed: bool, warnings: Vec<&str>) -> ValidationReportRecord {
    ValidationReportRecord {
        task_id: Some(TaskId::new(task_id)),
        passed,
        warnings: warnings.into_iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn validation_fails_if_task_not_validating() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Active),
        validation_report: Some(create_report(1, true, vec![])),
        proposed_status: TaskStatus::CompletedVerified,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("Validating state"))
    );
}

#[test]
fn validation_fails_if_report_missing() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: None,
        proposed_status: TaskStatus::CompletedVerified,
    };

    let decision = gate.evaluate(&request);
    assert!(matches!(
        decision,
        ValidationDecision::BlockedByMissingValidation { .. }
    ));
}

#[test]
fn validation_fails_if_report_task_mismatch() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(2, true, vec![])),
        proposed_status: TaskStatus::CompletedVerified,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("does not match task"))
    );
}

#[test]
fn validation_fails_if_report_failed() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, false, vec![])),
        proposed_status: TaskStatus::CompletedVerified,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("indicates failure"))
    );
}

#[test]
fn validation_fails_if_warnings_present_but_not_allowed() {
    let gate = DefaultValidationGate::new(false);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, true, vec!["warning"])),
        proposed_status: TaskStatus::CompletedWithWarnings,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("warnings are blocked"))
    );
}

#[test]
fn validation_succeeds_with_warnings_if_allowed() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, true, vec!["warning"])),
        proposed_status: TaskStatus::CompletedWithWarnings,
    };

    let decision = gate.evaluate(&request);
    assert_eq!(decision, ValidationDecision::AllowCompletion);
}

#[test]
fn validation_fails_if_warnings_present_but_status_is_verified() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, true, vec!["warning"])),
        proposed_status: TaskStatus::CompletedVerified,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("proposed status CompletedVerified but warnings are present"))
    );
}

#[test]
fn validation_fails_if_no_warnings_but_status_is_with_warnings() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, true, vec![])),
        proposed_status: TaskStatus::CompletedWithWarnings,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("proposed status CompletedWithWarnings but no warnings are present"))
    );
}

#[test]
fn validation_succeeds_without_warnings_verified() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, true, vec![])),
        proposed_status: TaskStatus::CompletedVerified,
    };

    let decision = gate.evaluate(&request);
    assert_eq!(decision, ValidationDecision::AllowCompletion);
}

#[test]
fn validation_fails_if_proposed_status_is_not_completion() {
    let gate = DefaultValidationGate::new(true);
    let request = ValidationRequest {
        task: create_task(1, TaskStatus::Validating),
        validation_report: Some(create_report(1, true, vec![])),
        proposed_status: TaskStatus::Active,
    };

    let decision = gate.evaluate(&request);
    assert!(
        matches!(decision, ValidationDecision::BlockedByPolicy { reason } if reason.contains("not a valid successful completion status"))
    );
}
