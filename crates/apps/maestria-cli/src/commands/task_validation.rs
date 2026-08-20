use anyhow::{Result, anyhow};
use maestria_domain::{
    ChangeTaskStatusInput, CompleteTaskInput, DomainInput, KernelState, RequestTaskValidation,
    TaskId, TaskStatus, ValidationReportId,
};
use maestria_governance::AutonomyProfile;
use std::path::PathBuf;
use std::time::Duration;

use crate::helpers;

/// Request a validation report for a task under the instance mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A validation request already
/// accepted by the runtime may still reach durable state; inspect durable
/// state before retrying an interrupted command.
pub async fn run_request_validation(instance_dir: PathBuf, task_id: u64) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await?;
    let task_id = TaskId::new(task_id);

    let result = async {
        let task_status = session
            .state()
            .tasks
            .get(&task_id)
            .ok_or_else(|| anyhow!("task {} not found", task_id))?
            .status;
        // Transition-path policy is owned by the domain transition graph (R28);
        // the CLI only walks the statuses the domain admits.
        let transition_plan = task_status.path_to_validating().ok_or_else(|| {
            anyhow!("cannot request validation from task status: {task_status:?}")
        })?;
        let start_event_index = session.state().event_log.len();

        for status in &transition_plan {
            let input = DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id,
                to: *status,
            });
            session.submit(input).await?;
        }
        if transition_plan.is_empty() && !session.recovery().validation_task_ids.contains(&task_id)
        {
            session
                .submit(DomainInput::RequestTaskValidation(RequestTaskValidation {
                    task_id,
                }))
                .await?;
        }
        wait_for_task_validation_report(
            &layout,
            task_id,
            start_event_index,
            Duration::from_secs(10),
        )
        .await
    }
    .await;

    let (_state, report_id, passed, warnings) = session.finish(result).await?;
    println!(
        "validation task={task_id} report={report_id} passed={passed} warnings={warnings:?}",
        task_id = task_id,
        report_id = report_id,
        passed = passed,
        warnings = warnings
    );

    Ok(())
}

/// Complete a task with a recorded validation report under the mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A completion command already
/// accepted by the runtime may still reach durable state; inspect durable
/// state before retrying an interrupted command.
pub async fn run_complete(
    instance_dir: PathBuf,
    task_id: u64,
    validation_report_id: u64,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await?;
    let task_id = TaskId::new(task_id);
    let validation_report_id = ValidationReportId::new(validation_report_id);

    let result = async {
        let state = session.state();
        let task = state
            .tasks
            .get(&task_id)
            .ok_or_else(|| anyhow!("task {} not found", task_id))?;
        if task.status != TaskStatus::Validating {
            anyhow::bail!(
                "task {task_id} is not in validating status (status={:?})",
                task.status
            );
        }
        let report = state
            .validation_reports
            .get(&validation_report_id)
            .ok_or_else(|| {
                anyhow!(
                    "validation report {validation_report_id} not found; request validation first"
                )
            })?;
        if report.task_id != Some(task_id) {
            anyhow::bail!(
                "validation report {validation_report_id} is not associated with task {task_id}"
            );
        }
        if !report.passed {
            anyhow::bail!("validation report {validation_report_id} failed; cannot complete task");
        }

        session
            .submit(DomainInput::CompleteTask(CompleteTaskInput {
                task_id,
                validation_report_id,
            }))
            .await?;
        wait_for_task_statuses(
            &layout,
            task_id,
            &[
                TaskStatus::CompletedVerified {
                    validation_report_id,
                },
                TaskStatus::CompletedWithWarnings {
                    validation_report_id,
                },
            ],
            Duration::from_secs(10),
        )
        .await
    }
    .await;

    let state = session.finish(result).await?;
    let task = state
        .tasks
        .get(&task_id)
        .ok_or_else(|| anyhow!("task {} not found after completion", task_id))?;
    println!(
        "completed task={task_id} status={status:?} validation_report={validation_report_id}",
        status = task.status,
        validation_report_id = validation_report_id
    );

    Ok(())
}

async fn wait_for_task_validation_report(
    layout: &maestria_core::InstanceLayout,
    task_id: TaskId,
    start_event_index: usize,
    timeout_budget: Duration,
) -> Result<(KernelState, ValidationReportId, bool, Vec<String>)> {
    let state = helpers::wait_for_kernel_state(
        layout,
        timeout_budget,
        format!("waiting for validation report for task {task_id}"),
        |state| {
            state.event_log.iter().skip(start_event_index).any(|event| {
                event
                    .event
                    .validation_report()
                    .is_some_and(|(_, task, _)| task == Some(task_id))
            })
        },
    )
    .await?;
    let (report_id, passed) = state
        .event_log
        .iter()
        .skip(start_event_index)
        .find_map(|event| match event.event.validation_report() {
            Some((report_id, Some(event_task_id), passed)) if event_task_id == task_id => {
                Some((report_id, passed))
            }
            _ => None,
        })
        .ok_or_else(|| anyhow!("validation report event vanished for task {task_id}"))?;
    let warnings = state
        .validation_reports
        .get(&report_id)
        .ok_or_else(|| anyhow!("validation report {report_id} missing after validation event"))?
        .warnings
        .clone();
    Ok((state, report_id, passed, warnings))
}

async fn wait_for_task_statuses(
    layout: &maestria_core::InstanceLayout,
    task_id: TaskId,
    expected: &[TaskStatus],
    timeout_budget: Duration,
) -> Result<KernelState> {
    helpers::wait_for_kernel_state(
        layout,
        timeout_budget,
        format!("waiting for task {task_id} completion"),
        |state| {
            state
                .tasks
                .get(&task_id)
                .is_some_and(|task| expected.contains(&task.status))
        },
    )
    .await
}
