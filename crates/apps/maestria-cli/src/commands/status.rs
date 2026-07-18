use crate::helpers;
use anyhow::Result;
use std::{path::PathBuf, time::Duration};

pub fn run(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let state = helpers::load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state for status",
    )?;
    println!("instance {}", layout.root.display());
    println!("database {}", layout.database_path.display());
    println!("full_text_index {}", layout.full_text_index_dir.display());
    println!("events {}", state.event_log.len());
    Ok(())
}
