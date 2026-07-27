use crate::recovery_inputs;
use maestria_domain::{ChangeTaskStatusInput, DomainInput, KernelState, OpenTaskInput, TaskId};

#[test]
fn pending_validations_derives_validation_for_validating_tasks_without_reports()
-> Result<(), maestria_domain::DomainError> {
    let mut state = KernelState::new();

    // Create a task that is in Validating state without a report
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(1),
        title: "Task 1".to_string(),
        artifact_id: None,
        priority: maestria_domain::TaskPriority::High,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(1),
        to: maestria_domain::TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(1),
        to: maestria_domain::TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(1),
        to: maestria_domain::TaskStatus::Validating,
    }))?;

    // Add a second task that is Validating but already has a report
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(2),
        title: "Task 2".to_string(),
        artifact_id: None,
        priority: maestria_domain::TaskPriority::Normal,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Validating,
    }))?;
    state.apply_input(DomainInput::RecordValidationReport(
        maestria_domain::RecordValidationReportInput {
            report_id: maestria_domain::ValidationReportId::new(1),
            task_id: Some(TaskId::new(2)),
            passed: true,
            warnings: vec![],
        },
    ))?;
    // Re-enter Validating after the first report; the historical report must
    // not satisfy the new validation cycle.
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Validating,
    }))?;

    // Add a third task that is not Validating
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(3),
        title: "Task 3".to_string(),
        artifact_id: None,
        priority: maestria_domain::TaskPriority::Normal,
    }))?;

    let recovery = recovery_inputs(&state);

    assert_eq!(
        recovery.run_validations.len(),
        2,
        "Should recover both tasks without a report for their current validation cycle"
    );

    let validation_task_ids: Vec<_> = recovery
        .run_validations
        .iter()
        .filter_map(|input| match input {
            DomainInput::RequestTaskValidation(request) => Some(request.task_id),
            _ => None,
        })
        .collect();
    assert_eq!(validation_task_ids, vec![TaskId::new(1), TaskId::new(2)]);

    Ok(())
}
