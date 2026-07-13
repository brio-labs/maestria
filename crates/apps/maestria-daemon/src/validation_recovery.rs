use maestria_domain::{DomainInput, KernelState};
use std::collections::BTreeSet;

/// Derive task-scoped validation requests for tasks left in `Validating`
/// without a replayed validation report.
pub fn pending_validations(state: &KernelState) -> Vec<DomainInput> {
    let mut inputs = Vec::new();
    let reports_for_tasks: BTreeSet<maestria_domain::TaskId> = state
        .validation_reports
        .values()
        .filter_map(|r| r.task_id)
        .collect();

    for (task_id, task) in &state.tasks {
        if task.status == maestria_domain::TaskStatus::Validating
            && !reports_for_tasks.contains(task_id)
        {
            inputs.push(DomainInput::RequestTaskValidation(
                maestria_domain::RequestTaskValidation { task_id: *task_id },
            ));
        }
    }
    inputs
}
