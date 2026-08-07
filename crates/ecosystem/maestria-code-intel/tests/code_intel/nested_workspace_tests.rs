use super::common::{
    assert_equivalent_to_full_rebuild, build_index, build_or_update, make_nested_workspaces,
    run_git, write_file,
};
use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use tempfile::tempdir;

#[test]
fn nested_workspaces_index_both_workspaces() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    let index = build_index(tmp.path(), "g1")?;
    assert_eq!(index.summary.package_count, 2);
    assert!(index.summary.packages.contains(&"crate_one".to_string()));
    assert!(index.summary.packages.contains(&"tool_x".to_string()));
    assert!(index.summary.workspace_warnings.is_empty());
    assert!(
        index.symbols.iter().any(|symbol| symbol.name == "root_add"),
        "root workspace symbol missing"
    );
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "nested_util"),
        "nested workspace symbol missing"
    );

    let result = index.query(
        CodeQuery::Symbol {
            pattern: "nested_util".to_string(),
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    let result = index.query(
        CodeQuery::Symbol {
            pattern: "root_add".to_string(),
        },
        20,
        |_: &SymbolRecord| Ok::<bool, Box<dyn Error>>(true),
    )?;
    assert_eq!(result.summary.matched, 1);
    Ok(())
}

#[test]
fn edit_in_either_workspace_is_incremental_and_equivalent() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // Edit inside the NESTED workspace.
    let nested_lib = root.join("rust/tools/tool_x/src/lib.rs");
    let mut source = fs::read_to_string(&nested_lib)?;
    source.push_str("\npub fn nested_extra() -> i32 { 7 }\n");
    fs::write(&nested_lib, source)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&index, root, true)?;
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "nested_extra"),
        "nested workspace edit not indexed"
    );

    // Edit inside the ROOT workspace member.
    let root_lib = root.join("crate_one/src/lib.rs");
    let mut source = fs::read_to_string(&root_lib)?;
    source.push_str("\npub fn root_extra() -> i32 { 9 }\n");
    fs::write(&root_lib, source)?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&index, root, true)?;
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "root_extra"),
        "root workspace edit not indexed"
    );
    Ok(())
}

#[test]
fn broken_nested_manifest_warns_and_indexes_healthy_workspaces() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    write_file(
        &root.join("rust/broken/Cargo.toml"),
        "[workspace]\nmembers = [\"does_not_exist\"]\n",
    )?;
    run_git(root, &["add", "."], "git add")?;
    run_git(
        root,
        &["commit", "-m", "add broken nested workspace"],
        "git commit",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(index.summary.packages.contains(&"crate_one".to_string()));
    assert!(index.summary.packages.contains(&"tool_x".to_string()));
    assert!(
        index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "nested_util"),
        "healthy nested workspace must still be indexed"
    );
    let warnings = &index.summary.workspace_warnings;
    assert!(!warnings.is_empty(), "expected discovery warnings");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("rust/broken")),
        "warning must name the broken workspace: {warnings:?}"
    );

    // A source edit with a broken nested manifest still rebuilds
    // incrementally; the warnings are carried forward and stay equivalent to
    // a full rebuild at the same repository state.
    let nested_lib = root.join("rust/tools/tool_x/src/lib.rs");
    let mut source = fs::read_to_string(&nested_lib)?;
    source.push_str("\npub fn nested_extra() -> i32 { 7 }\n");
    fs::write(&nested_lib, source)?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Incremental);
    assert_equivalent_to_full_rebuild(&index, root, true)?;
    assert!(!index.summary.workspace_warnings.is_empty());
    Ok(())
}

#[test]
fn repaired_nested_manifest_clears_warnings() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    write_file(
        &root.join("rust/broken/Cargo.toml"),
        "[workspace]\nmembers = [\"does_not_exist\"]\n",
    )?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(!index.summary.workspace_warnings.is_empty());

    // Repairing the broken workspace manifest is a manifest edit: it forces a
    // full rebuild whose discovery recomputes the warnings (now empty).
    write_file(
        &root.join("rust/broken/Cargo.toml"),
        "[workspace]\nmembers = []\n",
    )?;
    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index.summary.workspace_warnings.is_empty(),
        "warnings must be recomputed on full rebuild: {:?}",
        index.summary.workspace_warnings
    );
    Ok(())
}

#[test]
fn broken_root_manifest_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"missing_member\"]\n",
    )?;
    match RepositoryCodeIndex::build(tmp.path(), "g1") {
        Err(CodeIntelError::Command { .. }) => {}
        other => {
            return Err(
                format!("expected Command error for broken root manifest, got {other:?}").into(),
            );
        }
    }
    Ok(())
}

#[test]
fn nested_manifest_edit_changes_identity_and_forces_full() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    let indexed = RepositoryCodeIndex::load(&index_path)?;
    assert!(matches!(
        indexed.freshness()?,
        RepositoryFreshness::Current { .. }
    ));

    // An uncommitted nested manifest edit changes the worktree identity and
    // forces a full re-discovery.
    let manifest = root.join("rust/tools/Cargo.toml");
    let source = fs::read_to_string(&manifest)?;
    fs::write(&manifest, format!("{source}# updated member set\n"))?;
    let indexed = RepositoryCodeIndex::load(&index_path)?;
    assert!(matches!(
        indexed.freshness()?,
        RepositoryFreshness::Stale { .. }
    ));

    let (rebuilt, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(matches!(
        rebuilt.freshness()?,
        RepositoryFreshness::Current { .. }
    ));
    Ok(())
}

#[test]
fn new_auto_target_in_nested_workspace_forces_full() -> Result<(), Box<dyn Error>> {
    let tmp = make_nested_workspaces()?;
    let index_dir = tempdir()?;
    let index_path = index_dir.path().join("index.json");
    let candidates_path = index_dir.path().join("candidates.json");
    let root = tmp.path();

    let (_, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);

    // A new cargo auto-discovery test target inside the NESTED workspace
    // package needs a full rebuild even though no manifest changed.
    write_file(
        &root.join("rust/tools/tool_x/tests/nested_integ.rs"),
        "pub fn nested_integ_helper() -> i32 { 1 }\n",
    )?;

    let (index, mode) = build_or_update(&index_path, &candidates_path, root)?;
    assert_eq!(mode, RepositoryIndexBuildMode::Full);
    assert!(
        index.symbols.iter().any(|symbol| {
            symbol.provenance.file_path == "rust/tools/tool_x/tests/nested_integ.rs"
                && symbol.name == "nested_integ_helper"
        }),
        "nested auto target not extracted"
    );
    Ok(())
}
