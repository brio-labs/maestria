//! The candidate tree: every directory below a root, classified, with its
//! default policy, and its direct children.

use crate::classify::{Class, classify, default_policy};
use crate::policy::partition_by_child;
use crate::scan::{collect_files, dir_features_buckets, is_home_root};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// One directory in the candidate tree.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CandidateDir {
    pub path: PathBuf,
    pub class: Class,
    pub policy: crate::policy::IndexPolicy,
    pub file_count: usize,
    pub total_bytes: u64,
    pub children: Vec<CandidateDir>,
}

/// Scan `root` and classify every directory below it.
///
/// The root node itself is always `Recommended` with an everything policy;
/// its children are the top-level groups, each with the deterministic
/// classification. Ordering is deterministic: `partition_by_child` sorts by
/// (count desc, bytes desc).
pub fn scan_candidates(root: &Path) -> Result<CandidateDir> {
    let files = collect_files(root, true)?;
    let home_root = is_home_root(root);
    let node = build_node_generic(
        root,
        &files,
        home_root,
        crate::repo::REPO_DOC_EXTENSIONS.as_slice(),
        &crate::repo::REPO_CODE_EXTENSIONS[0..4],
    )?;
    let total_bytes = node.total_bytes;
    Ok(CandidateDir {
        path: root.to_path_buf(),
        class: Class::Recommended,
        policy: crate::policy::IndexPolicy::everything(),
        file_count: node.file_count,
        total_bytes,
        children: node.children,
    })
}

/// Shared candidate-tree recursion: classify `dir` from its `files` and
/// recurse into every direct child group, using `doc_extensions` /
/// `code_extensions` to compute the per-directory numerics. Used by both
/// the home-directory scan ([`scan_candidates`]) and the repository scan
/// (`repo.rs`) — one recursion, two extension buckets.
pub(crate) fn build_node_generic(
    dir: &Path,
    files: &[PathBuf],
    home_root: bool,
    doc_extensions: &[&str],
    code_extensions: &[&str],
) -> Result<CandidateDir> {
    let features = dir_features_buckets(dir, files, doc_extensions, code_extensions);
    let class = classify(&features, home_root, dir);
    let mut children = Vec::new();
    for (child, _, _, child_files) in partition_by_child(dir, files) {
        children.push(build_node_generic(
            &child,
            &child_files,
            home_root,
            doc_extensions,
            code_extensions,
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
