use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use std::path::PathBuf;

pub fn run(instance_dir: PathBuf) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let state = maestria_daemon::load_kernel_state(&layout).with_context(|| "load kernel state")?;
    println!("instance {}", layout.root.display());
    println!("database {}", layout.database_path.display());
    println!("full_text_index {}", layout.full_text_index_dir.display());
    println!("events {}", state.event_log.len());
    Ok(())
}
