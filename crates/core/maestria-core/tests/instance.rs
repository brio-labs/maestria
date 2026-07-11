use std::path::PathBuf;

use maestria_core::{InitInstanceInput, InstanceService};

#[test]
fn init_instance_returns_isolated_local_layout() -> Result<(), Box<dyn std::error::Error>> {
    let plan = InstanceService::init_instance(InitInstanceInput {
        root: PathBuf::from("/tmp/maestria/personal"),
    })?;

    assert_eq!(
        plan.layout.blobs_dir,
        PathBuf::from("/tmp/maestria/personal/blobs/sha256")
    );
    assert_eq!(
        plan.layout.full_text_index_dir,
        PathBuf::from("/tmp/maestria/personal/indexes/full-text")
    );
    assert!(plan.directories.contains(&plan.layout.active_tasks_dir));
    assert!(plan.manifest_contents.contains("schema_version=1"));

    Ok(())
}
