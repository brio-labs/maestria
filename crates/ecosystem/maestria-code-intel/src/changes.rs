//! Git-history-aware change delta computation for repository queries.
//!
//! The changed file set is derived from git metadata only: the porcelain
//! dirty set (staged plus worktree edits) and `git diff --name-only` between
//! a baseline commit and HEAD. No file contents are read anywhere in this
//! path, so a clean worktree costs the same as before the delta existed.

use crate::CodeIntelError;
use crate::identity::{discover_dirty_paths, git_output_allow_empty};
use crate::types::{CommitSha, RepositoryChangeDelta, RepositoryCodeIndex, SymbolRecord};
use std::collections::BTreeSet;
use std::path::Path;

/// The changed file set a `CodeQuery::Changed` matches against: the persisted
/// build-time delta when `since` is `None`, or the live git delta (diff
/// `since..HEAD` plus the current dirty set) when `Some`.
pub(crate) fn changed_file_set(
    index: &RepositoryCodeIndex,
    since: Option<&CommitSha>,
) -> Result<BTreeSet<String>, CodeIntelError> {
    let root = Path::new(&index.summary.repository_root);
    match since {
        None => Ok(index.summary.changed.files.iter().cloned().collect()),
        Some(commit) => {
            let mut files = diff_files_since(root, commit)?;
            files.extend(discover_dirty_paths(root)?);
            Ok(files)
        }
    }
}

/// Build-time changed file set: the porcelain dirty set plus the diff between
/// the replaced index's commit and HEAD. A `None` baseline (from-scratch full
/// build) contributes the dirty set only.
pub(crate) fn compute_delta_files(
    root: &Path,
    baseline: Option<&CommitSha>,
    dirty: &BTreeSet<String>,
) -> Result<BTreeSet<String>, CodeIntelError> {
    let mut files = dirty.clone();
    if let Some(commit) = baseline {
        files.extend(diff_files_since(root, commit)?);
    }
    Ok(files)
}

/// `record_id`s of symbols whose file is in `files`, ordered by file then
/// qualified name (deterministic regardless of input symbol order).
pub(crate) fn delta_symbol_ids(files: &BTreeSet<String>, symbols: &[SymbolRecord]) -> Vec<String> {
    let mut by_name: Vec<(&str, &str, &str)> = symbols
        .iter()
        .filter(|symbol| files.contains(&symbol.provenance.file_path))
        .map(|symbol| {
            (
                symbol.provenance.file_path.as_str(),
                symbol.qualified_name.as_str(),
                symbol.record_id.as_str(),
            )
        })
        .collect();
    by_name.sort_by(|left, right| left.0.cmp(right.0).then_with(|| left.1.cmp(right.1)));
    by_name
        .into_iter()
        .map(|(_, _, record_id)| record_id.to_string())
        .collect()
}

/// `git diff --name-only <baseline> HEAD` as a sorted set of relative paths.
fn diff_files_since(root: &Path, baseline: &CommitSha) -> Result<BTreeSet<String>, CodeIntelError> {
    let output = git_output_allow_empty(
        root,
        &["diff", "--name-only", baseline.as_str(), "HEAD"],
        "git diff --name-only",
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Whether `sha` is a plausible commit reference the delta computation can
/// hand to git: a full 40-hex SHA-1, a short hex prefix, or a `HEAD`-family
/// ref (`HEAD`, `HEAD~2`, `HEAD^`, ...). Garbage is rejected before any git
/// call so typos fail fast with a typed CLI error.
pub fn is_plausible_commit_sha(sha: &str) -> bool {
    let hex_len = sha.len();
    if (hex_len == 40 || (7..=39).contains(&hex_len))
        && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return true;
    }
    if let Some(rest) = sha
        .strip_prefix("HEAD~")
        .or_else(|| sha.strip_prefix("HEAD^"))
    {
        return rest.is_empty() || rest.bytes().all(|byte| byte.is_ascii_digit());
    }
    sha == "HEAD"
}

/// Build a completed delta from a file set and the indexed symbols.
pub(crate) fn build_delta(
    files: &BTreeSet<String>,
    symbols: &[SymbolRecord],
) -> RepositoryChangeDelta {
    RepositoryChangeDelta {
        files: files.iter().cloned().collect(),
        symbols: delta_symbol_ids(files, symbols),
    }
}

#[cfg(test)]
mod tests {
    use super::is_plausible_commit_sha;

    #[test]
    fn plausible_commit_validation() {
        assert!(is_plausible_commit_sha(&"a".repeat(40)));
        assert!(is_plausible_commit_sha("a1b2c3d"));
        assert!(is_plausible_commit_sha("HEAD"));
        assert!(is_plausible_commit_sha("HEAD~"));
        assert!(is_plausible_commit_sha("HEAD~2"));
        assert!(is_plausible_commit_sha("HEAD^1"));
        assert!(!is_plausible_commit_sha(""));
        assert!(!is_plausible_commit_sha("not-a-commit"));
        assert!(!is_plausible_commit_sha(&"a".repeat(6)));
        assert!(!is_plausible_commit_sha(&"g".repeat(40)));
        assert!(!is_plausible_commit_sha("HEAD~x"));
    }
}
