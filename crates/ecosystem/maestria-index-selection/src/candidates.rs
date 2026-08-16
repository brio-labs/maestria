//! The candidate tree: every directory below a root, classified, with its
//! default policy, and its direct children.

use crate::classify::{Class, classify, default_policy};
use crate::policy::{IndexPolicy, group_by_child};
use crate::scan::{DirFeatures, collect_files, dir_features, is_home_root};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// One directory in the candidate tree.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CandidateDir {
    pub path: PathBuf,
    pub class: Class,
    pub policy: IndexPolicy,
    pub file_count: usize,
    pub total_bytes: u64,
    pub children: Vec<CandidateDir>,
}

/// Scan `root` and classify every directory below it.
///
/// The root node itself is always `Recommended` with an everything policy;
/// its children are the top-level groups, each with the deterministic
/// classification. Ordering is deterministic: `group_by_child` sorts by
/// (count desc, bytes desc).
pub fn scan_candidates(root: &Path) -> Result<CandidateDir> {
    let files = collect_files(root, true)?;
    let home_root = is_home_root(root);
    let total_bytes = files
        .iter()
        .map(|file| std::fs::metadata(file).map_or(0, |metadata| metadata.len()))
        .sum();
    let node = build_node_generic(root, &files, home_root, dir_features)?;
    Ok(CandidateDir {
        path: root.to_path_buf(),
        class: Class::Recommended,
        policy: IndexPolicy::everything(),
        file_count: node.file_count,
        total_bytes,
        children: node.children,
    })
}

/// Shared candidate-tree recursion: classify `dir` from its `files` and
/// recurse into every direct child group, using `features` to compute the
/// per-directory numerics. Used by both the home-directory scan
/// ([`scan_candidates`]) and the repository scan (`repo.rs`) — one
/// recursion, two feature functions.
pub(crate) fn build_node_generic(
    dir: &Path,
    files: &[PathBuf],
    home_root: bool,
    features_fn: fn(&Path, &[PathBuf]) -> DirFeatures,
) -> Result<CandidateDir> {
    let features = features_fn(dir, files);
    let class = classify(&features, home_root, dir);
    let mut children = Vec::new();
    for (child, _, _) in group_by_child(dir, files) {
        let child_files: Vec<PathBuf> = files
            .iter()
            .filter(|file| file.starts_with(&child))
            .cloned()
            .collect();
        children.push(build_node_generic(
            &child,
            &child_files,
            home_root,
            features_fn,
        )?);
    }
    Ok(CandidateDir {
        path: dir.to_path_buf(),
        class,
        policy: default_policy(class),
        file_count: features.file_count,
        total_bytes: features.total_bytes,
        children,
    })
}

/// At most this many children per node survive the wire bound.
const MAX_TREE_CHILDREN: usize = 12;

/// Bound the candidate tree for the wire: keep the root, its children, and
/// their children (depth 2), with at most 12 children per node. Deeper
/// levels are dropped.
///
/// A scan of a large root (a home directory) otherwise produces a response
/// that exceeds the daemon protocol's message cap. Children are already
/// sorted by (count desc, bytes desc), so truncation deterministically
/// keeps the largest groups.
pub fn bound_candidate_tree(tree: &mut CandidateDir) {
    for child in tree.children.iter_mut() {
        for grandchild in child.children.iter_mut() {
            grandchild.children.clear();
        }
        child.children.truncate(MAX_TREE_CHILDREN);
    }
    tree.children.truncate(MAX_TREE_CHILDREN);
}
