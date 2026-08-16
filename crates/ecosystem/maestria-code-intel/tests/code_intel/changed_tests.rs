//! Changed-delta and git-history-aware query tests: the persisted summary
//! delta (porcelain dirty set plus the replaced index's baseline diff) and
//! `CodeQuery::Changed` with persisted (`since: None`) and live (`--since`)
//! semantics.

use super::common::{assert_equivalent_to_full_rebuild, make_workspace, run_git};
use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn build_or_update(
    index_path: &Path,
    candidates_path: &Path,
    root: &Path,
) -> Result<(RepositoryCodeIndex, RepositoryIndexBuildMode), Box<dyn Error>> {
    let (index, mode) = build_or_update_repository_index(
        index_path,
        candidates_path,
        root,
        "g1",
        &[],
        &RepositorySelection::everything(),
        &std::collections::BTreeMap::new(),
    )?;
    if !matches!(mode, RepositoryIndexBuildMode::Noop) {
        index.save(index_path)?;
    }
    Ok((index, mode))
}

fn authorize_all() -> impl FnMut(&SymbolRecord) -> Result<bool, Box<dyn Error>> {
    |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true)
}

fn lib_symbol_count(index: &RepositoryCodeIndex) -> usize {
    index
        .symbols
        .iter()
        .filter(|symbol| symbol.provenance.file_path == "crate_one/src/lib.rs")
        .count()
}

fn assert_matches_only_lib_file(result: &QueryResult) {
    assert!(
        result
            .records
            .iter()
            .all(|record| record.provenance.file_path == "crate_one/src/lib.rs"),
        "changed query must only return symbols of the edited file"
    );
}

/// Scenario A: an unstaged edit lands in the persisted changed delta and the
/// incremental rebuild matches a full rebuild including the changed section.
#[test]
fn changed_delta_includes_unstaged_edit() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(index.summary.changed.files().is_empty());
    assert!(index.summary.changed.symbols().is_empty());

    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn changed_unstaged() -> i32 { 3 }\n");
    fs::write(&lib_path, source)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_eq!(
        index.summary.changed.files().to_vec(),
        vec!["crate_one/src/lib.rs".to_string()]
    );
    assert_eq!(
        index.summary.changed.symbols().len(),
        lib_symbol_count(&index)
    );
    assert!(
        index
            .summary
            .changed
            .symbols()
            .iter()
            .all(|record_id| record_id.starts_with("crate_one/src/lib.rs:"))
    );
    assert_equivalent_to_full_rebuild(&index, root, true)?;

    // The persisted delta answers `changed` without any git call at query
    // time.
    let result = index.query(CodeQuery::Changed { since: None }, 100, authorize_all())?;
    assert_eq!(result.summary.matched, lib_symbol_count(&index));
    assert_matches_only_lib_file(&result);
    Ok(())
}

/// Scenario B: a staged edit is still in the delta (porcelain index status
/// column differs from a space).
#[test]
fn changed_delta_includes_staged_edit() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn changed_staged() -> i32 { 4 }\n");
    fs::write(&lib_path, source)?;
    run_git(root, &["add", "crate_one/src/lib.rs"], "git add")?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_eq!(
        index.summary.changed.files().to_vec(),
        vec!["crate_one/src/lib.rs".to_string()]
    );
    assert_equivalent_to_full_rebuild(&index, root, true)?;
    Ok(())
}

/// Scenario C: a committed edit lands in the delta through the baseline
/// (replaced index commit C)..HEAD diff on an otherwise clean worktree.
#[test]
fn changed_delta_includes_committed_edit() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn changed_committed() -> i32 { 5 }\n");
    fs::write(&lib_path, source)?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "commit edit"], "git commit")?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_eq!(
        index.summary.changed.files().to_vec(),
        vec!["crate_one/src/lib.rs".to_string()]
    );
    assert!(
        index
            .summary
            .changed
            .symbols()
            .iter()
            .any(|record_id| { record_id.contains(":function:changed_committed:") })
    );
    Ok(())
}

/// Scenario D: a clean worktree after a full build has an empty changed
/// section and every subsequent rebuild is a no-op.
#[test]
fn changed_delta_clean_worktree_is_empty_and_noop() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(index.summary.changed.files().is_empty());
    assert!(index.summary.changed.symbols().is_empty());

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    Ok(())
}

/// `--since <C>` after committing an edit at D computes the live delta
/// `diff(C..HEAD)` and returns exactly the edited file's symbols; edits made
/// after D (unstaged and staged) keep the file in the delta through the
/// current dirty set.
#[test]
fn changed_query_since_commit() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let base_commit = CommitSha::new(git_rev_parse(root, "HEAD")?);
    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn changed_committed() -> i32 { 6 }\n");
    fs::write(&lib_path, source)?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "commit edit"], "git commit")?;
    let (index, _) = build_or_update(&index_path, &candidates_path, root)?;

    let result = index.query(
        CodeQuery::Changed {
            since: Some(base_commit.clone()),
        },
        100,
        authorize_all(),
    )?;
    assert_eq!(result.summary.matched, lib_symbol_count(&index));
    assert_matches_only_lib_file(&result);

    // Unstaged edit after D: rebuild keeps the file in the persisted delta
    // through the dirty set, and the live query still resolves the same file.
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn changed_unstaged() -> i32 { 7 }\n");
    fs::write(&lib_path, source)?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    let result = index.query(
        CodeQuery::Changed {
            since: Some(base_commit.clone()),
        },
        100,
        authorize_all(),
    )?;
    assert_eq!(result.summary.matched, lib_symbol_count(&index));
    assert!(
        result
            .records
            .iter()
            .any(|record| record.name == "changed_unstaged"),
        "unstaged edit's symbol must be returned by the live changed query"
    );

    // Staging the already-indexed edit leaves the worktree content and dirty
    // set unchanged, so the rebuild is a no-op; the persisted changed section
    // still covers the file and the live query still resolves it.
    run_git(root, &["add", "crate_one/src/lib.rs"], "git add")?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    assert_eq!(
        index.summary.changed.files().to_vec(),
        vec!["crate_one/src/lib.rs".to_string()]
    );
    let result = index.query(
        CodeQuery::Changed {
            since: Some(base_commit.clone()),
        },
        100,
        authorize_all(),
    )?;
    assert_eq!(result.summary.matched, lib_symbol_count(&index));
    assert_matches_only_lib_file(&result);
    Ok(())
}

/// A live `--since` query on a clean, current index (empty dirty set) still
/// resolves the committed delta through git history.
#[test]
fn changed_query_since_on_clean_index() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let base_commit = CommitSha::new(git_rev_parse(root, "HEAD")?);
    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A second committed edit creates a diff the full build never saw.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn changed_committed() -> i32 { 8 }\n");
    fs::write(&lib_path, source)?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "commit edit"], "git commit")?;
    let (index, _) = build_or_update(&index_path, &candidates_path, root)?;

    let result = index.query(
        CodeQuery::Changed {
            since: Some(base_commit),
        },
        100,
        authorize_all(),
    )?;
    assert_eq!(result.summary.matched, lib_symbol_count(&index));
    assert!(
        result
            .records
            .iter()
            .any(|record| record.name == "changed_committed")
    );
    Ok(())
}

/// A garbage `--since` reference is rejected before any git call.
#[test]
fn changed_query_rejects_implausible_since() -> Result<(), Box<dyn Error>> {
    assert!(!maestria_code_intel::is_plausible_commit_sha(
        "not-a-commit"
    ));
    assert!(maestria_code_intel::is_plausible_commit_sha("HEAD~1"));
    Ok(())
}

fn git_rev_parse(root: &Path, reference: &str) -> Result<String, Box<dyn Error>> {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(["rev-parse", reference])
        .output()?;
    if !output.status.success() {
        return Err(format!("git rev-parse {reference} failed").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
