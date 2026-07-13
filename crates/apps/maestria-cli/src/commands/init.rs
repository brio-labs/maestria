use anyhow::Result;
use std::path::PathBuf;

pub fn run(instance_dir: PathBuf, read_roots: Vec<PathBuf>) -> Result<()> {
    let read_roots = if read_roots.is_empty() {
        vec![instance_dir.clone()]
    } else {
        read_roots
    };
    let plan = maestria_core::InstanceService::init_instance_with_roots(instance_dir, read_roots)?;
    for directory in &plan.directories {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    println!("initialized {}", plan.layout.root.display());
    println!("manifest {}", plan.manifest_path.display());
    Ok(())
}
