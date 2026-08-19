use maestria_core::InstanceLayout;
use maestria_domain::TaskId;

use super::TestDirectory;
use crate::commands::task::{
    TASK_WORKSPACE_SUBDIRECTORIES, create_task_workspace_directories, task_workspace_directory,
};

#[test]
fn task_workspace_directory_is_deterministic_and_created() -> Result<(), Box<dyn std::error::Error>>
{
    let instance_dir = TestDirectory::create()?;
    let layout = InstanceLayout::for_root(instance_dir.path());
    let task_id = TaskId::new(42);

    assert_eq!(
        task_workspace_directory(&layout, task_id),
        layout.active_tasks_dir.join("task_42")
    );

    create_task_workspace_directories(&layout, task_id)?;
    create_task_workspace_directories(&layout, task_id)?;

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
    Ok(())
}

#[test]
fn database_busy_matcher_identifies_lock_and_busy_errors() -> Result<(), Box<dyn std::error::Error>>
{
    use anyhow::anyhow;
    use maestria_storage_sqlite::db_retry::is_database_busy;
    let locked = anyhow!("database is locked");
    assert!(is_database_busy(&locked));

    let busy = anyhow!("database is busy");
    assert!(is_database_busy(&busy));

    let locked_variant = anyhow!("SQLite error: locked");
    assert!(is_database_busy(&locked_variant));

    let other = anyhow!("file not found");
    assert!(!is_database_busy(&other));
    Ok(())
}
