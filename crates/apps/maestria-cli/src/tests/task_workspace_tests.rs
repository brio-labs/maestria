use maestria_core::InstanceLayout;
use maestria_domain::TaskId;

use crate::helpers;

use super::collection_tests::TestDirectory;

// Duplicated from commands/task.rs for test isolation.
const TASK_WORKSPACE_SUBDIRECTORIES: [&str; 5] =
    ["context", "evidence", "drafts", "validation", "artifacts"];

fn task_workspace_directory(layout: &InstanceLayout, task_id: TaskId) -> std::path::PathBuf {
    layout
        .active_tasks_dir
        .join(format!("task_{}", task_id.value()))
}

fn create_task_workspace_directories(
    layout: &InstanceLayout,
    task_id: TaskId,
) -> anyhow::Result<()> {
    use anyhow::Context;
    use std::fs;

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

#[test]
fn task_workspace_directory_is_deterministic_and_created() {
    let instance_dir = TestDirectory::create();
    let layout = InstanceLayout::for_root(instance_dir.path());
    let task_id = TaskId::new(42);

    assert_eq!(
        task_workspace_directory(&layout, task_id),
        layout.active_tasks_dir.join("task_42")
    );

    create_task_workspace_directories(&layout, task_id)
        .expect("initial workspace creation succeeds");
    create_task_workspace_directories(&layout, task_id)
        .expect("repeated workspace creation succeeds");

    let task_directory = task_workspace_directory(&layout, task_id);
    assert!(
        task_directory.is_dir(),
        "task workspace directory was not created"
    );
    for subdirectory in TASK_WORKSPACE_SUBDIRECTORIES {
        assert!(
            task_directory.join(subdirectory).is_dir(),
            "missing task workspace child directory: {subdirectory}"
        );
    }
}

#[test]
fn is_db_locked_identifies_lock_and_busy_errors() {
    use anyhow::anyhow;

    let locked = anyhow!("database is locked");
    assert!(helpers::is_db_locked(&locked));

    let busy = anyhow!("database is busy");
    assert!(helpers::is_db_locked(&busy));

    let locked_variant = anyhow!("SQLite error: locked");
    assert!(helpers::is_db_locked(&locked_variant));

    let other = anyhow!("file not found");
    assert!(!helpers::is_db_locked(&other));
}
