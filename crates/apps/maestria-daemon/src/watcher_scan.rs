use anyhow::{Context, Result};
use maestria_core::{InstanceManifest, content_hash};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub(super) struct Observation {
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) hash: String,
}

pub(super) fn source_key(path: &Path) -> String {
    match path.canonicalize() {
        Ok(path) => path.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Scan manifest roots using `ignore::WalkBuilder` for gitignore/.ignore-aware
/// traversal. The walker respects `.gitignore`, `.ignore`, and hidden-file
/// conventions automatically.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn is_instance_path(path: &Path, normalized_instance_root: &Path) -> bool {
    normalize_path(path).starts_with(normalized_instance_root)
}

fn is_instance_internal_path(path: &Path, normalized_instance_root: &Path) -> bool {
    let normalized_path = normalize_path(path);
    let Some(relative) = normalized_path.strip_prefix(normalized_instance_root).ok() else {
        return false;
    };
    let Some(first) = relative.components().next() else {
        return false;
    };
    matches!(
        first,
        Component::Normal(name)
            if matches!(name.to_str(), Some("system" | "indexes" | "blobs" | "manifest.txt"))
    )
}

pub(super) fn scan_manifest(manifest: &InstanceManifest) -> Result<Vec<Observation>> {
    let mut observations = Vec::new();
    let instance_root = manifest.root.clone();
    let normalized_instance_root = normalize_path(&instance_root);

    for root in &manifest.read_roots {
        let root = root.clone();
        let normalized_root = normalize_path(&root);
        let exclude_instance = normalized_root != normalized_instance_root
            && normalized_instance_root.starts_with(&normalized_root);
        let normalized_instance_root = normalized_instance_root.clone();
        let walker = ignore::WalkBuilder::new(root)
            .filter_entry(move |entry| {
                if exclude_instance {
                    !is_instance_path(entry.path(), &normalized_instance_root)
                } else {
                    !is_instance_internal_path(entry.path(), &normalized_instance_root)
                }
            })
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(false)
            .require_git(false)
            .follow_links(false)
            .build();
        for result in walker {
            let entry = result?;
            if let Some(error) = entry.error() {
                return Err(anyhow::anyhow!(
                    "traversal error at {}: {error}",
                    entry.path().display()
                ));
            }
            let path = entry.path().to_path_buf();

            if !entry
                .file_type()
                .is_some_and(|ft| ft.is_file() && !ft.is_symlink())
            {
                continue;
            }

            if !manifest.allows_source(&path) {
                continue;
            }

            if !is_supported_file(&path) {
                continue;
            }

            let bytes =
                fs::read(&path).with_context(|| format!("read watched file {}", path.display()))?;
            observations.push(Observation {
                path,
                hash: content_hash(&bytes),
                bytes,
            });
        }
    }
    observations.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(observations)
}

fn is_supported_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt" | "rs" | "toml" | "json" | "yaml" | "yml" | "pdf")
    )
}
