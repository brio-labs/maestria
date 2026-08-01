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
    // Instance creation is startup policy owned by the daemon; the CLI entry
    // reuses it instead of copying directory/manifest handling.
    let layout = maestria_daemon::prepare_instance_with_roots(instance_dir, read_roots)?;
    println!("initialized {}", layout.root.display());
    println!("manifest {}", layout.manifest_path.display());
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
