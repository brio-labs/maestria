//! Repository index browsing: lazy directory expansion and file listing
//! for the Studio selection tree.
//!
//! The candidate scan is depth-bounded for the wire; these handlers let the
//! client expand one directory at a time (its classified subdirectories)
//! and list its direct files, so selection can go arbitrarily deep or stay
//! at the top. Both are read-only and spawn-blocked.

use super::index_services::{validate_scope_root, validate_within_root};
use crate::api::server::ApiContext;
use crate::api::{
    RepositoryIndexChildrenResponse, RepositoryIndexFile, RepositoryIndexFilesResponse,
};
use anyhow::{Result, anyhow};
use maestria_index_selection::{
    CandidateDir, classify, default_policy, group_by_child, repository_features,
};
use std::path::{Path, PathBuf};

/// At most this many direct files are carried per directory; larger
/// directories report `truncated` so the client can tell the user.
const MAX_DIRECT_FILES: usize = 200;

/// Resolve a repository-relative directory path against the validated root.
/// A single leading slash is tolerated (path stripping artifacts).
fn resolve_directory(root: &Path, path: &str) -> Result<PathBuf> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Ok(root.to_path_buf());
    }
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(anyhow!(
            "directory path {path:?} must be repository-relative without `..`"
        ));
    }
    let joined = root.join(relative);
    validate_within_root(&joined, root)?;
    Ok(joined)
}

/// The direct subdirectories of one repository directory, each classified
/// with empty children (expanded on demand).
///
/// # Cancellation
/// Cancelling drops the in-flight scan; no state is changed.
pub(super) async fn children(
    context: &ApiContext,
    root: String,
    path: String,
) -> Result<RepositoryIndexChildrenResponse> {
    validate_scope_root(context, &root)?;
    let root_path = PathBuf::from(&root);
    let dir = resolve_directory(&root_path, &path)?;
    let response = tokio::task::spawn_blocking(move || {
        let files = maestria_index_selection::collect_repository_files(&dir)?;
        let mut children = Vec::new();
        for (child, _, _) in group_by_child(&dir, &files) {
            // `group_by_child` groups by the first path component, which is
            // a FILE for the directory's direct files; those belong to the
            // file listing, not to the subdirectory expansion.
            if !child.is_dir() {
                continue;
            }
            let child_files: Vec<PathBuf> = files
                .iter()
                .filter(|file| file.starts_with(&child))
                .cloned()
                .collect();
            let features = repository_features(&child, &child_files);
            let class = classify(&features, false, &child);
            children.push(CandidateDir {
                path: child,
                class,
                policy: default_policy(class),
                file_count: features.file_count,
                total_bytes: features.total_bytes,
                children: Vec::new(),
            });
        }
        Ok::<_, anyhow::Error>(RepositoryIndexChildrenResponse {
            root,
            path,
            children,
        })
    })
    .await
    .map_err(|error| anyhow!("repository index children task failed: {error}"))??;
    Ok(response)
}

/// The direct files of one repository directory, bounded for the wire.
///
/// # Cancellation
/// Cancelling drops the in-flight listing; no state is changed.
pub(super) async fn files(
    context: &ApiContext,
    root: String,
    path: String,
) -> Result<RepositoryIndexFilesResponse> {
    validate_scope_root(context, &root)?;
    let root_path = PathBuf::from(&root);
    let dir = resolve_directory(&root_path, &path)?;
    let response = tokio::task::spawn_blocking(move || {
        let mut listing = Vec::new();
        let entries = std::fs::read_dir(&dir)
            .map_err(|error| anyhow!("list repository directory {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                anyhow!(
                    "read repository directory entry in {}: {error}",
                    dir.display()
                )
            })?;
            let entry_path = entry.path();
            if !entry_path.is_file() {
                continue;
            }
            let name = entry_path
                .file_name()
                .and_then(|name| name.to_str())
                .map_or("", |name| name);
            if name.starts_with('.')
                || maestria_index_selection::is_privacy_excluded_path(&entry_path)
            {
                continue;
            }
            let relative = entry_path
                .strip_prefix(&root_path)
                .map_err(|_| anyhow!("repository file escaped its root"))?
                .to_string_lossy()
                .into_owned();
            let size = std::fs::metadata(&entry_path).map_or(0, |metadata| metadata.len());
            listing.push(RepositoryIndexFile {
                path: relative,
                size,
                kind: file_kind(&entry_path),
            });
        }
        listing.sort_by(|left, right| left.path.cmp(&right.path));
        let truncated = listing.len() > MAX_DIRECT_FILES;
        listing.truncate(MAX_DIRECT_FILES);
        Ok::<_, anyhow::Error>(RepositoryIndexFilesResponse {
            root,
            path,
            files: listing,
            truncated,
        })
    })
    .await
    .map_err(|error| anyhow!("repository index files task failed: {error}"))??;
    Ok(response)
}

/// The population bucket of a repository file.
fn file_kind(path: &Path) -> String {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                "Cargo.toml"
                    | "Cargo.lock"
                    | "pyproject.toml"
                    | "setup.cfg"
                    | "setup.py"
                    | "package.json"
            )
        })
    {
        return "manifest".to_string();
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some(extension) if maestria_index_selection::REPO_CODE_EXTENSIONS.contains(&extension) => {
            "code".to_string()
        }
        Some(extension) if maestria_index_selection::REPO_DOC_EXTENSIONS.contains(&extension) => {
            "doc".to_string()
        }
        _ => "other".to_string(),
    }
}

#[cfg(test)]
#[path = "repository_index_browse_tests.rs"]
mod tests;
