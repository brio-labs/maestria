use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::{
    ArtifactId, DomainInput, EvidenceId, KernelState, LinkEvidenceToTaskInput, OpenTaskInput, Task,
    TaskId,
};
use maestria_governance::AutonomyProfile;
use std::{fs, path::PathBuf, time::Duration};

pub use super::task_validation::{run_complete, run_request_validation};
use crate::cli_types::CliTaskPriority;
use crate::helpers;
pub(crate) const TASK_WORKSPACE_SUBDIRECTORIES: [&str; 5] =
    ["context", "evidence", "drafts", "validation", "artifacts"];

/// Open a task under the instance mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A task-open command already
/// accepted by the runtime may still reach durable state; inspect durable
/// state before retrying an interrupted command.
pub async fn run_start(
    instance_dir: PathBuf,
    title: String,
    priority: CliTaskPriority,
    artifact_id: Option<u64>,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await?;

    let result = async {
        let task_id = next_task_id(session.state());
        create_task_workspace_directories(&layout, task_id)?;

        let input = DomainInput::OpenTask(OpenTaskInput {
            task_id,
            title,
            priority: priority.into(),
            artifact_id: artifact_id.map(ArtifactId::new),
        });
        session.submit(input).await?;
        let state = wait_for_task_in_state(&layout, task_id, Duration::from_secs(2)).await?;

        state
            .tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| anyhow!("task {} was not persisted", task_id))
    }
    .await;

    let task = session.finish(result).await?;
    println!(
        "task={} title={} status={:?} priority={:?}",
        task.id, task.title, task.status, task.priority
    );

    Ok(())
}

/// Link evidence to a task under the instance mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A link command already accepted by
/// the runtime may still reach durable state; inspect durable state before
/// retrying an interrupted command.
pub async fn run_add_evidence(instance_dir: PathBuf, task_id: u64, evidence_id: u64) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await?;

    let result = async {
        let state = session.state();
        let task_id = TaskId::new(task_id);
        let evidence_id = EvidenceId::new(evidence_id);

        if !state.tasks.contains_key(&task_id) {
            return Err(anyhow!("task {} not found", task_id));
        }
        if !state.evidences.contains_key(&evidence_id) {
            return Err(anyhow!("evidence {} not found", evidence_id));
        }

        let input = DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
            task_id,
            evidence_id,
        });
        session.submit(input).await?;
        let state =
            wait_for_task_evidence_link(&layout, task_id, evidence_id, Duration::from_secs(2))
                .await?;

        let task = state
            .tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| anyhow!("task {} not found after persistence", task_id))?;
        if !task.evidence_ids.contains(&evidence_id) {
            return Err(anyhow!(
                "evidence {} was not linked to task {} after persistence",
                evidence_id,
                task_id
            ));
        }

        Ok(task)
    }
    .await;

    let task = session.finish(result).await?;
    println!(
        "linked evidence={evidence_id} to task={task_id} status={:?}",
        task.status
    );

    Ok(())
}

pub fn run_show(instance_dir: PathBuf, task_id: Option<u64>) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let state = maestria_daemon::load_kernel_state(&layout).with_context(|| "load kernel state")?;

    if let Some(requested) = task_id {
        let requested = TaskId::new(requested);
        let task = state
            .tasks
            .get(&requested)
            .ok_or_else(|| anyhow!("task {} not found", requested))?;
        print_task(task);
        return Ok(());
    }

    if state.tasks.is_empty() {
        println!("no tasks");
        return Ok(());
    }

    for task in state.tasks.values() {
        print_task(task);
    }

    Ok(())
}

fn print_task(task: &Task) {
    print!(
        "task={} status={:?} priority={:?} title='{}'",
        task.id, task.status, task.priority, task.title
    );

    if let Some(report_id) = task.status.validation_report_id() {
        print!(" validation_report={report_id}");
    }

    if !task.artifact_ids.is_empty() {
        print!(" artifacts={:?}", task.artifact_ids);
    }

    if !task.evidence_ids.is_empty() {
        print!(" evidence={:?}", task.evidence_ids);
    }

    println!();
}

fn next_task_id(state: &maestria_domain::KernelState) -> TaskId {
    state
        .tasks
        .iter()
        .next_back()
        .map_or(TaskId::new(1), |(id, _)| TaskId::new(id.value() + 1))
}

pub(crate) fn task_workspace_directory(layout: &InstanceLayout, task_id: TaskId) -> PathBuf {
    layout
        .active_tasks_dir
        .join(format!("task_{}", task_id.value()))
}

pub(crate) fn create_task_workspace_directories(
    layout: &InstanceLayout,
    task_id: TaskId,
) -> Result<()> {
    let task_directory = task_workspace_directory(layout, task_id);
    fs::create_dir_all(&task_directory).with_context(|| {
        format!(
            "failed to create task workspace {} for task {}",
            task_directory.display(),
            task_id
        )
    })?;

    for subdirectory in TASK_WORKSPACE_SUBDIRECTORIES {
        let path = task_directory.join(subdirectory);
        fs::create_dir_all(&path).with_context(|| {
            format!(
                "failed to create task {task_id} {} directory {}",
                subdirectory,
                path.display()
            )
        })?;
    }

    Ok(())
}

async fn wait_for_task_in_state(
    layout: &InstanceLayout,
    task_id: TaskId,
    timeout_budget: Duration,
) -> Result<KernelState> {
    helpers::wait_for_kernel_state(
        layout,
        timeout_budget,
        format!("waiting for task {task_id} persistence"),
        |state| state.tasks.contains_key(&task_id),
    )
    .await
}

async fn wait_for_task_evidence_link(
    layout: &InstanceLayout,
    task_id: TaskId,
    evidence_id: EvidenceId,
    timeout_budget: Duration,
) -> Result<KernelState> {
    helpers::wait_for_kernel_state(
        layout,
        timeout_budget,
        format!("waiting for evidence link persistence for task {task_id}"),
        |state| {
            state
                .tasks
                .get(&task_id)
                .is_some_and(|task| task.evidence_ids.contains(&evidence_id))
        },
    )
    .await
}
