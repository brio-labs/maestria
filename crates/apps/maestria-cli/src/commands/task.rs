use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::{
    ArtifactId, DomainInput, EvidenceId, KernelState, LinkEvidenceToTaskInput, OpenTaskInput, Task,
    TaskId,
};
use maestria_governance::AutonomyProfile;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use tokio::time::{sleep, timeout};

use crate::cli_types::CliTaskPriority;
use crate::helpers;

const TASK_WORKSPACE_SUBDIRECTORIES: [&str; 5] =
    ["context", "evidence", "drafts", "validation", "artifacts"];

pub async fn run_start(
    instance_dir: PathBuf,
    title: String,
    priority: CliTaskPriority,
    artifact_id: Option<u64>,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let state = load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before task start",
    )
    .await?;
    let task_id = next_task_id(&state);
    create_task_workspace_directories(&layout, task_id)?;

    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, state, AutonomyProfile::TrustedWorkspace)
            .with_context(|| "build runtime")?;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
        let input = DomainInput::OpenTask(OpenTaskInput {
            task_id,
            title,
            priority: priority.into(),
            artifact_id: artifact_id.map(ArtifactId::new),
        });
        input_tx
            .send(input)
            .await
            .map_err(|error| anyhow!("failed to queue task open input: {error}"))?;
        wait_for_task_in_state(&layout, task_id, Duration::from_secs(2)).await
    }
    .await;

    shutdown_token.cancel();
    let join_result = runtime_task.await;
    let state = result?;
    join_result.with_context(|| "runtime loop join failed")?;

    let task = state
        .tasks
        .get(&task_id)
        .cloned()
        .ok_or_else(|| anyhow!("task {} was not persisted", task_id))?;

    println!(
        "task={} title={} status={:?} priority={:?}",
        task.id, task.title, task.status, task.priority
    );

    Ok(())
}

pub async fn run_add_evidence(instance_dir: PathBuf, task_id: u64, evidence_id: u64) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let state = load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before add-evidence",
    )
    .await?;
    let task_id = TaskId::new(task_id);
    let evidence_id = EvidenceId::new(evidence_id);

    if !state.tasks.contains_key(&task_id) {
        return Err(anyhow!("task {} not found", task_id));
    }
    if !state.evidences.contains_key(&evidence_id) {
        return Err(anyhow!("evidence {} not found", evidence_id));
    }

    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, state, AutonomyProfile::TrustedWorkspace)
            .with_context(|| "build runtime")?;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));
    let result = async {
        let input = DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
            task_id,
            evidence_id,
        });
        input_tx
            .send(input)
            .await
            .map_err(|error| anyhow!("failed to queue link-evidence input: {error}"))?;
        wait_for_task_evidence_link(&layout, task_id, evidence_id, Duration::from_secs(2)).await
    }
    .await;

    shutdown_token.cancel();
    let join_result = runtime_task.await;
    let state = result?;
    join_result.with_context(|| "runtime loop join failed")?;

    let task = state
        .tasks
        .get(&task_id)
        .ok_or_else(|| anyhow!("task {} not found after persistence", task_id))?;
    if !task.evidence_ids.contains(&evidence_id) {
        return Err(anyhow!(
            "evidence {} was not linked to task {} after persistence",
            evidence_id,
            task_id
        ));
    }

    println!(
        "linked evidence={evidence_id} to task={task_id} status={:?}",
        task.status
    );

    Ok(())
}

pub fn run_show(instance_dir: PathBuf, task_id: Option<u64>) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
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

    if let Some(report_id) = task.validation_report_id {
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

fn task_workspace_directory(layout: &InstanceLayout, task_id: TaskId) -> PathBuf {
    layout
        .active_tasks_dir
        .join(format!("task_{}", task_id.value()))
}

fn create_task_workspace_directories(layout: &InstanceLayout, task_id: TaskId) -> Result<()> {
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
    use std::sync::Mutex;

    let last_error = Arc::new(Mutex::new(None::<String>));
    let last_error_for_wait = last_error.clone();
    timeout(timeout_budget, async move {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| "load kernel state while waiting for task persistence")
            {
                Ok(state) => {
                    if state.tasks.contains_key(&task_id) {
                        return Ok(state);
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    return Err(error);
                }
            }
        }
    })
    .await
    .map_err(|_| {
        let detail = last_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .map_or_else(String::new, |error| format!(" {error}"));
        anyhow!("timed out while waiting for task {task_id} persistence{detail}")
    })?
}

async fn wait_for_task_evidence_link(
    layout: &InstanceLayout,
    task_id: TaskId,
    evidence_id: EvidenceId,
    timeout_budget: Duration,
) -> Result<KernelState> {
    let last_error = Arc::new(std::sync::Mutex::new(None::<String>));
    let last_error_for_wait = last_error.clone();
    timeout(timeout_budget, async move {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| "load kernel state while waiting for evidence link persistence")
            {
                Ok(state) => {
                    if let Some(task) = state.tasks.get(&task_id)
                        && task.evidence_ids.contains(&evidence_id)
                    {
                        return Ok(state);
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    return Err(error);
                }
            }
        }
    })
    .await
    .map_err(|_| {
        let detail = last_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .map_or_else(String::new, |error| format!(" {error}"));
        anyhow!("timed out while waiting for evidence {evidence_id} link to task {task_id}{detail}")
    })?
}

async fn load_kernel_state_with_retry(
    layout: &InstanceLayout,
    timeout_budget: Duration,
    context: &'static str,
) -> Result<KernelState> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout).with_context(|| context) {
                Ok(state) => return Ok(state),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while {context}"))?
}
