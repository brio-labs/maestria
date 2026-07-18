use anyhow::{Context, Result, anyhow};
use maestria_domain::{
    ChangeTaskStatusInput, CompleteTaskInput, DomainEvent, DomainInput, KernelState,
    RequestTaskValidation, TaskId, TaskStatus, ValidationReportId,
};
use maestria_governance::AutonomyProfile;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::{sleep, timeout};

use crate::helpers;

pub async fn run_request_validation(instance_dir: PathBuf, task_id: u64) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;

    let state = helpers::load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before request validation",
    )?;

    let task_id = TaskId::new(task_id);
    let task_status = state
        .tasks
        .get(&task_id)
        .ok_or_else(|| anyhow!("task {} not found", task_id))?
        .status;

    let transition_plan = task_validation_transition_plan(task_status)?;
    let start_event_index = state.event_log.len();

    let (runtime, input_tx, input_rx, shutdown_token) = timeout(Duration::from_secs(5), async {
        loop {
            match maestria_daemon::build_runtime(
                &layout,
                state.clone(),
                AutonomyProfile::TrustedWorkspace,
            ) {
                Ok(runtime) => break Ok(runtime),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => break Err(error).with_context(|| "build runtime"),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while building runtime"))??;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
        for status in &transition_plan {
            let input = DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id,
                to: *status,
            });
            input_tx.send(input).await.map_err(|error| {
                anyhow!(
                    "failed to queue validation transition {:?}: {error}",
                    status
                )
            })?;
        }
        if transition_plan.is_empty() {
            input_tx
                .send(DomainInput::RequestTaskValidation(RequestTaskValidation {
                    task_id,
                }))
                .await
                .map_err(|error| {
                    anyhow!("failed to queue explicit task validation request: {error}")
                })?;
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

    shutdown_token.cancel();
    let join_result = runtime_task.await;
    let (_state, report_id, passed, warnings) = result?;
    join_result.with_context(|| "runtime loop join failed")?;

    println!(
        "validation task={task_id} report={report_id} passed={passed} warnings={warnings:?}",
        task_id = task_id,
        report_id = report_id,
        passed = passed,
        warnings = warnings
    );

    Ok(())
}

pub async fn run_complete(
    instance_dir: PathBuf,
    task_id: u64,
    validation_report_id: u64,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;

    let state = helpers::load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before task completion",
    )?;
    let task_id = TaskId::new(task_id);
    let validation_report_id = ValidationReportId::new(validation_report_id);

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
            anyhow!("validation report {validation_report_id} not found; request validation first")
        })?;
    if report.task_id != Some(task_id) {
        anyhow::bail!(
            "validation report {validation_report_id} is not associated with task {task_id}"
        );
    }
    if !report.passed {
        anyhow::bail!("validation report {validation_report_id} failed; cannot complete task");
    }

    let (runtime, input_tx, input_rx, shutdown_token) = timeout(Duration::from_secs(5), async {
        loop {
            match maestria_daemon::build_runtime(
                &layout,
                state.clone(),
                AutonomyProfile::TrustedWorkspace,
            ) {
                Ok(runtime) => break Ok(runtime),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => break Err(error).with_context(|| "build runtime"),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while building runtime"))??;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
        let input = DomainInput::CompleteTask(CompleteTaskInput {
            task_id,
            validation_report_id,
        });
        input_tx
            .send(input)
            .await
            .map_err(|error| anyhow!("failed to queue task completion: {error}"))?;
        wait_for_task_statuses(
            &layout,
            task_id,
            &[
                TaskStatus::CompletedVerified,
                TaskStatus::CompletedWithWarnings,
            ],
            Duration::from_secs(10),
        )
        .await
    }
    .await;

    shutdown_token.cancel();
    let join_result = runtime_task.await;
    let state = result?;
    join_result.with_context(|| "runtime loop join failed")?;

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

fn task_validation_transition_plan(task_status: TaskStatus) -> Result<Vec<TaskStatus>> {
    Ok(match task_status {
        TaskStatus::Draft => vec![TaskStatus::Open, TaskStatus::Active, TaskStatus::Validating],
        TaskStatus::Open => vec![TaskStatus::Active, TaskStatus::Validating],
        TaskStatus::Active => vec![TaskStatus::Validating],
        TaskStatus::Blocked => vec![TaskStatus::Active, TaskStatus::Validating],
        TaskStatus::Validating => Vec::new(),
        status => anyhow::bail!("cannot request validation from task status: {status:?}"),
    })
}

async fn wait_for_task_validation_report(
    layout: &maestria_core::InstanceLayout,
    task_id: TaskId,
    start_event_index: usize,
    timeout_budget: Duration,
) -> Result<(KernelState, ValidationReportId, bool, Vec<String>)> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout).with_context(|| {
                format!("load kernel state while waiting for validation report for task {task_id}")
            }) {
                Ok(state) => {
                    if let Some((report_id, passed)) =
                        state.event_log.iter().skip(start_event_index).find_map(
                            |event| match &event.event {
                                DomainEvent::ValidationReportCreated {
                                    report_id,
                                    task_id: Some(event_task_id),
                                    passed,
                                    ..
                                } if *event_task_id == task_id => Some((*report_id, *passed)),
                                _ => None,
                            },
                        )
                    {
                        let warnings = state
                            .validation_reports
                            .get(&report_id)
                            .ok_or_else(|| {
                                anyhow!(
                                    "validation report {report_id} missing after validation event"
                                )
                            })?
                            .warnings
                            .clone();
                        return Ok((state, report_id, passed, warnings));
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while waiting for validation report for task {task_id}"))?
}

async fn wait_for_task_statuses(
    layout: &maestria_core::InstanceLayout,
    task_id: TaskId,
    expected: &[TaskStatus],
    timeout_budget: Duration,
) -> Result<KernelState> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout).with_context(|| {
                format!("load kernel state while waiting for task {task_id} completion")
            }) {
                Ok(state) => {
                    if let Some(task) = state.tasks.get(&task_id)
                        && expected.contains(&task.status)
                    {
                        return Ok(state);
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| {
        anyhow!(
            "timed out waiting for task {task_id} to reach status in {:?}",
            expected
        )
    })?
}
