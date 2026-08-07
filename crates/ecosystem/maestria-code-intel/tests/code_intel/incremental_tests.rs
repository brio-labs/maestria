use super::common::{init_git, make_workspace, run_git, write_file};
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
    let (index, mode) =
        build_or_update_repository_index(index_path, candidates_path, root, "g1", &[])?;
    if !matches!(mode, RepositoryIndexBuildMode::Noop) {
        index.save(index_path)?;
    }
    Ok((index, mode))
}

fn assert_equivalent_to_full_rebuild(
    incremental: &RepositoryCodeIndex,
    root: &Path,
) -> Result<(), Box<dyn Error>> {
    let fresh = RepositoryCodeIndex::build(root, "g1")?;
    assert_eq!(
        incremental.summary.package_count,
        fresh.summary.package_count
    );
    assert_eq!(incremental.summary.target_count, fresh.summary.target_count);
    assert_eq!(incremental.summary.symbol_count, fresh.summary.symbol_count);
    assert_eq!(incremental.summary.file_count, fresh.summary.file_count);
    assert_eq!(incremental.summary.packages, fresh.summary.packages);
    assert_eq!(
        incremental.summary.relation_summary,
        fresh.summary.relation_summary
    );
    assert_eq!(incremental.file_contexts, fresh.file_contexts);
    assert_eq!(incremental.relations, fresh.relations);
    let mut incremental_symbols = incremental.symbols.clone();
    incremental_symbols.sort_by(|left, right| left.record_id.cmp(&right.record_id));
    let mut fresh_symbols = fresh.symbols.clone();
    fresh_symbols.sort_by(|left, right| left.record_id.cmp(&right.record_id));
    assert_eq!(incremental_symbols, fresh_symbols);
    Ok(())
}

#[test]
fn incremental_edit_equals_full_rebuild() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Append a function (shifts lines) to a tracked file.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn appended() -> i32 { 7 }\n");
    fs::write(&lib_path, source)?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&incremental, root)?;

    // Edit the body of an existing function in the same file.
    let source = fs::read_to_string(&lib_path)?;
    let edited = source.replace("a + b", "a - b");
    fs::write(&lib_path, edited)?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&incremental, root)?;
    Ok(())
}

#[test]
fn incremental_removes_deleted_file() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    fs::remove_file(root.join("crate_one/src/lib.rs"))?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        incremental
            .symbols
            .iter()
            .all(|symbol| symbol.provenance.file_path != "crate_one/src/lib.rs")
    );
    assert!(incremental.relations.iter().all(|relation| {
        !relation
            .source_record_id
            .starts_with("crate_one/src/lib.rs:")
            && !relation
                .target_record_id
                .starts_with("crate_one/src/lib.rs:")
    }));
    assert!(
        !incremental
            .file_contexts
            .contains_key("crate_one/src/lib.rs")
    );
    assert_eq!(incremental.summary.symbol_count, 0);
    assert_eq!(incremental.summary.file_count, 0);
    assert_eq!(incremental.summary.relation_summary.total_relations, 0);
    Ok(())
}

#[test]
fn incremental_handles_git_rename() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    // Add a module file and commit it.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub mod sub;\n");
    fs::write(&lib_path, source)?;
    write_file(
        &root.join("crate_one/src/sub.rs"),
        "pub fn sub_helper() -> i32 { 1 }\n",
    )?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "add sub module"], "git commit")?;

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Rename the module file and point lib.rs at the new name.
    run_git(
        root,
        &["mv", "crate_one/src/sub.rs", "crate_one/src/sub2.rs"],
        "git mv",
    )?;
    let source = fs::read_to_string(&lib_path)?;
    let edited = source.replace("pub mod sub;", "pub mod sub2;");
    fs::write(&lib_path, edited)?;

    let (incremental, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        incremental
            .symbols
            .iter()
            .any(
                |symbol| symbol.provenance.file_path == "crate_one/src/sub2.rs"
                    && symbol.name == "sub_helper"
            )
    );
    assert!(
        incremental
            .symbols
            .iter()
            .all(|symbol| symbol.provenance.file_path != "crate_one/src/sub.rs")
    );
    assert!(incremental.relations.iter().all(|relation| {
        !relation
            .source_record_id
            .starts_with("crate_one/src/sub.rs:")
            && !relation
                .target_record_id
                .starts_with("crate_one/src/sub.rs:")
    }));
    assert!(
        !incremental
            .file_contexts
            .contains_key("crate_one/src/sub.rs")
    );
    assert_equivalent_to_full_rebuild(&incremental, root)?;
    Ok(())
}

#[test]
fn incremental_module_edit_then_undeclare() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub mod sub;\n");
    fs::write(&lib_path, source)?;
    let sub_path = root.join("crate_one/src/sub.rs");
    write_file(&sub_path, "pub fn sub_helper() -> i32 { 1 }\n")?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "add sub module"], "git commit")?;

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Edit only the module file (parent stays clean): it is re-extracted as
    // its own derivation root while keeping its module parent linkage.
    write_file(
        &sub_path,
        "pub fn sub_helper() -> i32 { 1 }\npub fn sub_extra() -> i32 { 3 }\n",
    )?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index.symbols.iter().any(
            |symbol| symbol.provenance.file_path == "crate_one/src/sub.rs"
                && symbol.name == "sub_extra"
        )
    );
    assert_eq!(
        index
            .file_contexts
            .get("crate_one/src/sub.rs")
            .and_then(|record| record.parent.as_deref()),
        Some("crate_one/src/lib.rs")
    );
    assert_equivalent_to_full_rebuild(&index, root)?;

    // Remove the `mod` declaration: the module must be dropped even though it
    // was re-extracted by the previous rebuild.
    let source = fs::read_to_string(&lib_path)?;
    let edited = source.replace("\npub mod sub;\n", "\n");
    fs::write(&lib_path, edited)?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index
            .symbols
            .iter()
            .all(|symbol| symbol.provenance.file_path != "crate_one/src/sub.rs")
    );
    assert!(!index.file_contexts.contains_key("crate_one/src/sub.rs"));
    assert_equivalent_to_full_rebuild(&index, root)?;
    Ok(())
}

#[test]
fn staged_edit_is_reparsed() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A staged edit is invisible to the worktree status column but changes
    // what a full rebuild would extract.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn staged_fn() -> i32 { 5 }\n");
    fs::write(&lib_path, source)?;
    run_git(root, &["add", "crate_one/src/lib.rs"], "git add")?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index.symbols.iter().any(
            |symbol| symbol.provenance.file_path == "crate_one/src/lib.rs"
                && symbol.name == "staged_fn"
        )
    );
    assert_equivalent_to_full_rebuild(&index, root)?;
    Ok(())
}

#[test]
fn reverted_edit_is_reparsed() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Index an edit, then revert it: the worktree becomes porcelain-clean and
    // blob-identical to HEAD, but the extracted content is stale.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn reverted_fn() -> i32 { 6 }\n");
    fs::write(&lib_path, source)?;
    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);

    run_git(
        root,
        &["checkout", "--", "crate_one/src/lib.rs"],
        "git checkout",
    )?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index
            .symbols
            .iter()
            .all(|symbol| symbol.name != "reverted_fn")
    );
    assert_equivalent_to_full_rebuild(&index, root)?;
    Ok(())
}

#[test]
fn committed_edit_stays_incremental() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Index an edit, then commit it: the worktree is clean and the blob map
    // changed, but the extracted content is exactly the committed content, so
    // the rebuild only rewrites identity fields.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub fn committed_fn() -> i32 { 8 }\n");
    fs::write(&lib_path, source)?;
    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);

    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "commit edit"], "git commit")?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index.symbols.iter().any(
            |symbol| symbol.provenance.file_path == "crate_one/src/lib.rs"
                && symbol.name == "committed_fn"
        )
    );
    assert_equivalent_to_full_rebuild(&index, root)?;
    Ok(())
}

#[test]
fn cargo_toml_change_forces_full() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Add a new workspace member: dir + manifest + source, and register it.
    fs::create_dir_all(root.join("crate_two/src"))?;
    write_file(
        &root.join("crate_two/Cargo.toml"),
        r#"
[package]
name = "crate_two"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.join("crate_two/src/lib.rs"),
        "pub fn two() -> i32 { 2 }\n",
    )?;
    let manifest = fs::read_to_string(root.join("Cargo.toml"))?;
    let edited = manifest.replace(
        "members = [\"crate_one\"]",
        "members = [\"crate_one\", \"crate_two\"]",
    );
    fs::write(root.join("Cargo.toml"), edited)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(index.summary.packages.contains(&"crate_two".to_string()));
    assert!(
        index.symbols.iter().any(
            |symbol| symbol.provenance.file_path == "crate_two/src/lib.rs" && symbol.name == "two"
        )
    );
    Ok(())
}

#[test]
fn unchanged_repo_is_noop() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    let before = fs::read(&index_path)?;

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Noop);
    let after = fs::read(&index_path)?;
    assert_eq!(before, after);
    Ok(())
}

#[test]
fn missing_file_contexts_falls_back_to_full() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    // An index persisted without the required `file_contexts` field cannot
    // even load; the rebuild falls back to Full and rewrites it.
    let legacy = RepositoryCodeIndex::build(root, "g1")?;
    legacy.save(&index_path)?;
    let json = fs::read_to_string(&index_path)?;
    let mut value: serde_json::Value = serde_json::from_str(&json)?;
    value
        .as_object_mut()
        .ok_or("missing index object")?
        .remove("file_contexts");
    fs::write(&index_path, serde_json::to_vec_pretty(&value)?)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    let fresh = RepositoryCodeIndex::build(root, "g1")?;
    assert_eq!(index.symbols, fresh.symbols);
    assert_eq!(index.relations, fresh.relations);
    assert_eq!(index.file_contexts, fresh.file_contexts);
    assert_eq!(index.summary, fresh.summary);
    Ok(())
}

#[test]
fn new_orphan_file_is_incremental_and_equivalent() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // An untracked `.rs` file that no dirty file can re-derive is unreachable
    // for extraction: a full build would not extract it either, so the
    // incremental rebuild stays equivalent without a full fallback.
    write_file(
        &root.join("crate_one/src/extra.rs"),
        "pub fn extra() -> i32 { 1 }\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    let fresh = RepositoryCodeIndex::build(root, "g1")?;
    assert_eq!(index.symbols, fresh.symbols);
    assert_eq!(index.relations, fresh.relations);
    assert_eq!(index.file_contexts, fresh.file_contexts);
    assert_eq!(index.summary, fresh.summary);
    Ok(())
}

#[test]
fn new_auto_test_target_forces_full() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A new integration-test target is auto-discovered by cargo without any
    // manifest change, so only a full rebuild can pick it up.
    write_file(
        &root.join("crate_one/tests/integ.rs"),
        "pub fn integ_helper() -> i32 { 1 }\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index
            .symbols
            .iter()
            .any(
                |symbol| symbol.provenance.file_path == "crate_one/tests/integ.rs"
                    && symbol.name == "integ_helper"
            )
    );
    Ok(())
}

#[test]
fn gitignored_extracted_file_reparsed() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    // Declare the module in lib.rs (extraction only reaches declared modules),
    // commit a .gitignore covering generated.rs, and add the ignored file:
    // it is extracted by the walk-based module discovery but tracked by no
    // file set.
    let lib_path = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&lib_path)?;
    source.push_str("\npub mod generated;\n");
    fs::write(&lib_path, source)?;
    write_file(&root.join(".gitignore"), "generated.rs\n")?;
    let generated = root.join("crate_one/src/generated.rs");
    write_file(&generated, "pub fn generated() -> i32 { 1 }\n")?;
    run_git(root, &["add", "."], "git add")?;
    run_git(
        root,
        &["commit", "-m", "add gitignore and generated module"],
        "git commit",
    )?;

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    write_file(
        &generated,
        "pub fn generated() -> i32 { 1 }\npub fn generated_two() -> i32 { 2 }\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert!(
        index
            .symbols
            .iter()
            .any(
                |symbol| symbol.provenance.file_path == "crate_one/src/generated.rs"
                    && symbol.name == "generated_two"
            )
    );
    assert_equivalent_to_full_rebuild(&index, root)?;
    Ok(())
}
