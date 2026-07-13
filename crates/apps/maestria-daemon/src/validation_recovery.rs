use maestria_domain::{DomainEvent, DomainInput, KernelState, TaskId, TaskStatus};

/// Derive task-scoped validation requests for tasks left in `Validating`
/// without a report created after their latest validation transition.
pub fn pending_validations(state: &KernelState) -> Vec<DomainInput> {
    state
        .tasks
        .iter()
        .filter_map(|(task_id, task)| {
            (task.status == TaskStatus::Validating
                && !has_current_validation_report(state, *task_id))
            .then_some(DomainInput::RequestTaskValidation(
                maestria_domain::RequestTaskValidation { task_id: *task_id },
            ))
        })
        .collect()
}

fn has_current_validation_report(state: &KernelState, task_id: TaskId) -> bool {
    let transition_index = state.event_log.iter().rposition(|envelope| {
        matches!(
            envelope.event,
            DomainEvent::TaskStatusChanged {
                task_id: event_task_id,
                to: TaskStatus::Validating,
                ..
            } if event_task_id == task_id
        )
    });
    let Some(transition_index) = transition_index else {
        return false;
    };
    state
        .event_log
        .iter()
        .skip(transition_index + 1)
        .any(|envelope| {
            matches!(
                envelope.event,
                DomainEvent::ValidationReportCreated {
                    report_id: _,
                    task_id: Some(event_task_id),
                    ..
                } if event_task_id == task_id
            )
        })
}
