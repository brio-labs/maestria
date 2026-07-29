use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn run(instance_dir: PathBuf, read_roots: Vec<PathBuf>) -> Result<()> {
    let instance_dir = canonicalize_new_path(&instance_dir)?;
    let read_roots = if read_roots.is_empty() {
        vec![instance_dir.clone()]
    } else {
        read_roots
            .into_iter()
            .map(|root| {
                root.canonicalize()
                    .with_context(|| format!("canonicalize read root {}", root.display()))
            })
            .collect::<Result<Vec<_>>>()?
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

fn canonicalize_new_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("resolve new path {}", path.display()))?;
    let existing_ancestor = absolute
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .with_context(|| format!("find existing ancestor for {}", absolute.display()))?;
    let canonical_ancestor = existing_ancestor
        .canonicalize()
        .with_context(|| format!("canonicalize ancestor {}", existing_ancestor.display()))?;
    let unresolved = absolute
        .strip_prefix(existing_ancestor)
        .with_context(|| format!("resolve suffix for {}", absolute.display()))?;
    Ok(canonical_ancestor.join(unresolved))
}
