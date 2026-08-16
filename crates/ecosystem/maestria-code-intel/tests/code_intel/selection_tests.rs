//! Selection-scoped build and incremental rebuild tests: records, identity,
//! delta, and freshness scoped to the selected directories; policy gating;
//! and the full/incremental/noop transitions on selection changes.

use maestria_code_intel::*;
use maestria_index_selection::IndexPolicy;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

use super::common::{run_git, write_file};

/// Two-crate workspace fixture: `crates/one` and `crates/two`, each with a
/// `src/lib.rs`, committed as a git repository.
fn make_two_crate_workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        r#"
[workspace]
members = ["crates/one", "crates/two"]
"#,
    )?;
    write_file(
        &root.path().join("crates/one/Cargo.toml"),
        r#"
[package]
name = "one"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("crates/one/src/lib.rs"),
        "pub fn one() {}\n",
    )?;
    write_file(
        &root.path().join("crates/two/Cargo.toml"),
        r#"
[package]
name = "two"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("crates/two/src/lib.rs"),
        "pub fn two() {}\n",
    )?;
    run_git(
        root.path(),
        &["init", "--initial-branch", "main"],
        "git init",
    )?;
    run_git(
        root.path(),
        &["config", "user.email", "ci@example.com"],
        "git config user.email",
    )?;
    run_git(
        root.path(),
        &["config", "user.name", "CI"],
        "git config user.name",
    )?;
    run_git(root.path(), &["add", "."], "git add")?;
    run_git(root.path(), &["commit", "-m", "fixture init"], "git commit")?;
    Ok(root)
}

fn build_selected(
    index_path: &Path,
    candidates_path: &Path,
    root: &Path,
    selection: &[&str],
    policies: &BTreeMap<String, IndexPolicy>,
) -> Result<(RepositoryCodeIndex, RepositoryIndexBuildMode), Box<dyn Error>> {
    let selection =
        RepositorySelection::try_new(selection.iter().map(|path| (*path).to_string()).collect())?;
    let (index, mode) = build_or_update_repository_index(
        index_path,
        candidates_path,
        root,
        "g1",
        &[],
        &selection,
        policies,
    )?;
    if !matches!(mode, RepositoryIndexBuildMode::Noop) {
        index.save(index_path)?;
    }
    Ok((index, mode))
}

fn policy(max_file_bytes: u64) -> IndexPolicy {
    IndexPolicy {
        max_file_bytes,
        skip_generated: false,
        skip_minified: false,
    }
}

/// Assert two indexes built with the same selection are equivalent: same
/// symbols (by record id), contexts, relations, counts, and changed delta.
fn assert_selected_equivalence(incremental: &RepositoryCodeIndex, fresh: &RepositoryCodeIndex) {
    assert_eq!(
        incremental.summary.package_count,
        fresh.summary.package_count
    );
    assert_eq!(incremental.summary.target_count, fresh.summary.target_count);
    assert_eq!(incremental.summary.symbol_count, fresh.summary.symbol_count);
    assert_eq!(incremental.summary.file_count, fresh.summary.file_count);
    assert_eq!(incremental.summary.packages, fresh.summary.packages);
    assert_eq!(
        incremental.summary.selected_paths,
        fresh.summary.selected_paths
    );
    assert_eq!(
        incremental.summary.selection_policies,
        fresh.summary.selection_policies
    );
    assert_eq!(incremental.file_contexts, fresh.file_contexts);
    assert_eq!(incremental.relations, fresh.relations);
    let mut incremental_symbols = incremental.symbols.clone();
    incremental_symbols.sort_by(|left, right| left.record_id.cmp(&right.record_id));
    let mut fresh_symbols = fresh.symbols.clone();
    fresh_symbols.sort_by(|left, right| left.record_id.cmp(&right.record_id));
    assert_eq!(incremental_symbols, fresh_symbols);
    assert_eq!(incremental.summary.changed, fresh.summary.changed);
}

#[test]
fn selected_full_build_scopes_records() -> Result<(), Box<dyn Error>> {
    let tmp = make_two_crate_workspace()?;
    let index_dir = tempdir()?;
    let root = tmp.path();

    let (index, mode) = build_selected(
        &index_dir.path().join("index.json"),
        &index_dir.path().join("candidates.json"),
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert_eq!(index.summary.selected_paths, vec!["crates/one".to_string()]);
    assert_eq!(index.summary.selection_policies, BTreeMap::new());
    assert_eq!(index.summary.packages, vec!["one".to_string()]);
    assert_eq!(index.summary.target_count, 1);

    let one_files = index
        .symbols
        .iter()
        .map(|symbol| symbol.provenance.file_path.as_str())
        .collect::<Vec<_>>();
    assert!(
        !one_files.is_empty(),
        "selected crate must contribute symbols"
    );
    assert!(
        one_files.iter().all(|file| file.starts_with("crates/one/")),
        "symbols must only come from the selection: {one_files:?}"
    );
    assert!(
        index
            .file_contexts
            .keys()
            .all(|file| file.starts_with("crates/one/")),
        "contexts must only come from the selection"
    );
    assert!(
        index
            .symbols
            .iter()
            .all(|symbol| symbol.provenance.file_path != "crates/two/src/lib.rs"),
        "unselected crate must not contribute symbols"
    );
    assert!(
        index.relations.iter().all(|relation| {
            relation
                .source_provenance
                .file_path
                .starts_with("crates/one/")
                && relation
                    .target_provenance
                    .file_path
                    .starts_with("crates/one/")
        }),
        "relations must only connect surviving records"
    );
    Ok(())
}

#[test]
fn selected_incremental_equals_full_rebuild() -> Result<(), Box<dyn Error>> {
    let tmp = make_two_crate_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    let lib = root.join("crates/one/src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("pub fn one_more() -> i32 { 1 }\n");
    fs::write(&lib, source)?;

    let (incremental, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        incremental
            .symbols
            .iter()
            .any(|symbol| symbol.name == "one_more"),
        "the edited selected file must be re-extracted"
    );

    let (fresh, mode) = build_selected(
        &index_dir.path().join("fresh.json"),
        &index_dir.path().join("fresh-candidates.json"),
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert_selected_equivalence(&incremental, &fresh);
    Ok(())
}

#[test]
fn outside_selection_edit_is_noop() -> Result<(), Box<dyn Error>> {
    let tmp = make_two_crate_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (index, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    let before = index.symbols.len();

    // Edit outside the selection: nothing to re-extract, no delta entry.
    let lib = root.join("crates/two/src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("pub fn two_more() {}\n");
    fs::write(&lib, source)?;

    let (index, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    assert_eq!(index.symbols.len(), before);
    assert!(
        index.summary.changed.files().is_empty(),
        "no changed files inside the selection"
    );
    Ok(())
}

#[test]
fn selection_change_forces_full_rebuild() -> Result<(), Box<dyn Error>> {
    let tmp = make_two_crate_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Widening the selection forces a full rebuild and adds the crate.
    let (index, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one", "crates/two"],
        &BTreeMap::new(),
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert_eq!(
        index.summary.selected_paths,
        vec!["crates/one", "crates/two"]
    );
    assert_eq!(index.summary.packages, vec!["one", "two"]);
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.provenance.file_path == "crates/two/src/lib.rs"),
        "the newly selected crate must be indexed"
    );

    // A policy change on an identical selection also forces a full rebuild.
    let policies = BTreeMap::from([("crates/one".to_string(), policy(1024))]);
    let (_, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one", "crates/two"],
        &policies,
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    Ok(())
}

#[test]
fn policy_gates_oversized_files_and_persists() -> Result<(), Box<dyn Error>> {
    let tmp = make_two_crate_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    // Both lib.rs files are 15 bytes; a 10-byte cap gates them out.
    let policies = BTreeMap::from([("crates".to_string(), policy(10))]);
    let (index, mode) =
        build_selected(&index_path, &candidates_path, root, &["crates"], &policies)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index.symbols.is_empty(),
        "gated files must not be extracted"
    );
    assert!(index.file_contexts.is_empty());
    assert_eq!(index.summary.selection_policies, policies);

    // Identical selection + policies: Noop, no forced migration.
    let (_, mode) = build_selected(&index_path, &candidates_path, root, &["crates"], &policies)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    Ok(())
}

#[test]
fn gated_file_dropped_on_incremental_fix() -> Result<(), Box<dyn Error>> {
    let tmp = make_two_crate_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    // 15-byte lib.rs under a 64-byte cap: indexed.
    let policies = BTreeMap::from([("crates/one".to_string(), policy(64))]);
    let (index, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &policies,
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index.symbols.iter().any(|symbol| symbol.name == "one"),
        "the small file must be extracted under the cap"
    );

    // Grow the file past the cap; the same selection + policies rebuild
    // drops it through the incremental gate (identity changed, extraction
    // gated), matching a fresh full build.
    let lib = root.join("crates/one/src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("pub fn grown() {}\npub fn grown_more() {}\npub fn grown_more_still() {}\n");
    fs::write(&lib, source)?;

    let (incremental, mode) = build_selected(
        &index_path,
        &candidates_path,
        root,
        &["crates/one"],
        &policies,
    )?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        incremental.symbols.is_empty(),
        "the oversized file must be dropped on rebuild"
    );
    assert!(
        !incremental
            .file_contexts
            .contains_key("crates/one/src/lib.rs"),
        "the oversized file's context must be dropped on rebuild"
    );

    let (fresh, _) = build_selected(
        &index_dir.path().join("fresh.json"),
        &index_dir.path().join("fresh-candidates.json"),
        root,
        &["crates/one"],
        &policies,
    )?;
    assert_selected_equivalence(&incremental, &fresh);
    Ok(())
}
