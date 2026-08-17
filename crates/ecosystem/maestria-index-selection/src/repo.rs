//! Repository-mode candidate scan: collect the code/document population of
//! a git repository and classify every directory with the same numeric
//! rules as the home scan, minus the home-name noise rule.

use crate::candidates::build_node_generic;
use crate::classify::Class;
use crate::policy::IndexPolicy;
use crate::scan::{DirFeatures, dir_features_buckets, is_privacy_excluded_path};
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Repository code extensions. MUST stay equal to
/// `maestria-code-intel`'s `KNOWN_SOURCE_EXTENSIONS`
/// (`crates/ecosystem/maestria-code-intel/src/language/mod.rs`): the
/// repository scan and the code index must agree on what a code file is.
pub const REPO_CODE_EXTENSIONS: [&str; 8] = ["rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// Repository document extensions.
pub const REPO_DOC_EXTENSIONS: [&str; 3] = ["md", "markdown", "txt"];

/// Directory names skipped wholesale: version-control and build/dependency
/// output that never contributes code or docs worth classifying.
const REPO_SKIPPED_DIRS: [&str; 6] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "__pycache__",
];

/// Collect the repository population under `root`, in deterministic
/// (sorted) order.
///
/// Mirror of `scan.rs::collect_files`: `ignore::WalkBuilder` with hidden,
/// ignore-file, and gitignore rules honored, no symlink following, and
/// privacy exclusions applied. Additionally skips `REPO_SKIPPED_DIRS` and
/// `*.egg-info` directories. Every file in the population is counted, so
/// the generated-dump rule (which needs ≥200 non-doc/non-code files) can
/// fire; [`repository_features`] buckets the doc/code shares over
/// `REPO_DOC_EXTENSIONS`/`REPO_CODE_EXTENSIONS`, and code-intel decides
/// which extensions it actually indexes.
pub fn collect_repository_files(root: &Path) -> Result<Vec<PathBuf>> {
    if is_privacy_excluded_path(root) {
        return Err(anyhow::anyhow!(
            "repository path is excluded by privacy policy: {}",
            root.display()
        ));
    }
    if !root.is_dir() {
        return Err(anyhow::anyhow!(
            "repository path does not exist: {}",
            root.display()
        ));
    }

    let mut files = BTreeSet::new();
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false);
    builder.filter_entry(|entry| {
        if entry.depth() == 0 {
            return true;
        }
        let entry_path = entry.path();
        if is_privacy_excluded_path(entry_path) {
            return false;
        }
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
            && is_repo_skipped_dir(entry_path)
        {
            return false;
        }
        true
    });

    for result in builder.build() {
        let entry = result?;
        let entry_path = entry.path();
        if let Some(error) = entry.error() {
            return Err(anyhow::anyhow!(
                "repository traversal failed at {}: {error}",
                entry_path.display()
            ));
        }
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_symlink())
        {
            continue;
        }
        if !entry_path.is_file() {
            continue;
        }
        files.insert(entry_path.to_path_buf());
    }

    Ok(files.into_iter().collect())
}

fn is_repo_skipped_dir(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("", |name| name);
    REPO_SKIPPED_DIRS.contains(&name) || name.ends_with(".egg-info")
}

/// Numeric features of a repository directory's population, over the
/// repository doc/code buckets.
pub fn repository_features(_dir: &Path, files: &[PathBuf]) -> DirFeatures {
    dir_features_buckets(_dir, files, &REPO_DOC_EXTENSIONS, &REPO_CODE_EXTENSIONS)
}

/// Scan `root` (a repository) and classify every directory below it.
///
/// The root node is always `Recommended` with an everything policy; its
/// children are classified with the same deterministic numeric rules as
/// [`crate::scan_candidates`] but with `home_root = false` (no name-based
/// noise rule; numeric rules only).
pub fn scan_repository_candidates(root: &Path) -> Result<crate::CandidateDir> {
    let files = collect_repository_files(root)?;
    let total_bytes = files
        .iter()
        .map(|file| std::fs::metadata(file).map_or(0, |metadata| metadata.len()))
        .sum();
    let node = build_node_generic(root, &files, false, repository_features)?;
    Ok(crate::CandidateDir {
        path: root.to_path_buf(),
        class: Class::Recommended,
        policy: IndexPolicy::everything(),
        file_count: node.file_count,
        total_bytes,
        children: node.children,
    })
}
