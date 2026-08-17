//! Repository selection release-contract tests: candidate listing shape and
//! selection-scoped build/search/noop/incremental behavior.

use maestria_cli::test_support::{TempDir, assert_init_ok, assert_ok, write_file};
use std::error::Error;
use std::fs;
use std::path::Path;

fn search_code_symbol(
    instance_path: &str,
    pattern: &str,
) -> Result<(usize, Vec<String>), Box<dyn Error>> {
    let stdout = assert_ok(&["search", "-i", instance_path, "code", "symbol", pattern])?;
    let value: serde_json::Value = serde_json::from_str(&stdout)?;
    let matched = value["summary"]["matched"]
        .as_u64()
        .ok_or("missing matched")? as usize;
    let records = value["records"]
        .as_array()
        .ok_or("missing records")?
        .iter()
        .filter_map(|record| record["record_id"].as_str().map(str::to_string))
        .collect();
    Ok((matched, records))
}

/// Two-crate workspace fixture: `crates/one` and `crates/two` packages with
/// `src/lib.rs` each, committed as a git repository.
fn make_two_crate_repo(repo: &Path) -> Result<(), Box<dyn Error>> {
    write_file(
        repo,
        "Cargo.toml",
        r#"
[workspace]
members = ["crates/one", "crates/two"]
"#,
    )?;
    write_file(
        repo,
        "crates/one/Cargo.toml",
        r#"
[package]
name = "one"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(repo, "crates/one/src/lib.rs", "pub fn one() -> i32 { 1 }\n")?;
    write_file(
        repo,
        "crates/two/Cargo.toml",
        r#"
[package]
name = "two"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(repo, "crates/two/src/lib.rs", "pub fn two() -> i32 { 2 }\n")?;
    maestria_test_support::run_git(repo, &["init", "--initial-branch", "main"], "git init")?;
    maestria_test_support::run_git(
        repo,
        &["config", "user.email", "ci@example.com"],
        "git config user.email",
    )?;
    maestria_test_support::run_git(repo, &["config", "user.name", "CI"], "git config user.name")?;
    maestria_test_support::run_git(repo, &["add", "."], "git add")?;
    maestria_test_support::run_git(repo, &["commit", "-m", "fixture init"], "git commit")?;
    Ok(())
}

#[test]
fn repository_index_list_prints_bounded_candidate_tree() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-list-repo")?;
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_two_crate_repo(repo.path())?;

    // --list needs no instance: it prints the bounded classified tree.
    let stdout = assert_ok(&["index", "repository", "--list", &repo_path])?;
    let tree: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(tree["class"], "Recommended");
    assert_eq!(tree["file_count"], 5);
    let children = tree["children"].as_array().ok_or("missing children")?;
    let crates = children
        .iter()
        .find(|child| {
            child["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("crates"))
        })
        .ok_or("crates child missing from candidate tree")?;
    assert_eq!(crates["class"], "Recommended");
    assert_eq!(crates["file_count"], 4);
    Ok(())
}

#[test]
fn repository_index_selection_scopes_build_and_search() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-selection-repo")?;
    let instance = TempDir::new("maestria-release-selection-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_two_crate_repo(repo.path())?;
    assert_init_ok(&instance_path, &repo_path)?;

    // --include scopes the build to crates/one: mode=full, the summary
    // records the selection, and search sees only the selected symbols.
    let stdout = assert_ok(&[
        "index",
        "-i",
        &instance_path,
        "repository",
        "--include",
        "crates/one",
        &repo_path,
    ])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    assert!(
        stdout.contains("selected_paths=crates/one"),
        "expected selected_paths line: {stdout}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    assert_eq!(
        summary["selected_paths"][0].as_str(),
        Some("crates/one"),
        "summary must record the selection"
    );
    assert_eq!(
        summary["packages"][0].as_str(),
        Some("one"),
        "only the selected package may be indexed"
    );

    let (matched, records) = search_code_symbol(&instance_path, "one")?;
    assert!(
        matched >= 1 && records.iter().any(|record| record.contains("one")),
        "selected symbol must be searchable: {records:?}"
    );
    let (matched, records) = search_code_symbol(&instance_path, "two")?;
    assert_eq!(
        matched, 0,
        "unselected symbols must not be searchable: {records:?}"
    );

    // An edit outside the selection is a no-op.
    let lib = repo.path().join("crates/two/src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("pub fn two_more() -> i32 { 3 }\n");
    fs::write(&lib, source)?;
    let stdout = assert_ok(&[
        "index",
        "-i",
        &instance_path,
        "repository",
        "--include",
        "crates/one",
        &repo_path,
    ])?;
    assert!(stdout.contains("mode=noop"), "expected noop: {stdout}");

    // An edit inside the selection rebuilds incrementally.
    let lib = repo.path().join("crates/one/src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("pub fn one_more() -> i32 { 4 }\n");
    fs::write(&lib, source)?;
    let stdout = assert_ok(&[
        "index",
        "-i",
        &instance_path,
        "repository",
        "--include",
        "crates/one",
        &repo_path,
    ])?;
    assert!(
        stdout.contains("mode=incremental"),
        "expected incremental rebuild: {stdout}"
    );
    let (matched, records) = search_code_symbol(&instance_path, "one_more")?;
    assert!(
        matched == 1 && records.iter().any(|record| record.contains("one_more")),
        "new selected symbol not searchable: {records:?}"
    );
    Ok(())
}

#[test]
fn repository_index_default_selects_recommended_and_skips_noise() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-default-repo")?;
    let instance = TempDir::new("maestria-release-default-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_two_crate_repo(repo.path())?;
    // A generated dump: 300 tiny same-extension files that must be Noise.
    for index in 0..300 {
        write_file(
            repo.path(),
            &format!("vendor-dump/asset{index:03}.json"),
            "{\"k\":1}",
        )?;
    }
    maestria_test_support::run_git(repo.path(), &["add", "."], "git add")?;
    maestria_test_support::run_git(repo.path(), &["commit", "-m", "add dump"], "git commit")?;
    assert_init_ok(&instance_path, &repo_path)?;

    // The default (no selection flags) indexes the Recommended directories
    // only: crates, never the dump.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    assert!(
        stdout.contains("selected_paths=crates"),
        "default must select Recommended dirs: {stdout}"
    );
    assert!(
        !stdout.contains("vendor-dump"),
        "default must exclude the Noise dump: {stdout}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    assert_eq!(
        summary["selected_paths"][0].as_str(),
        Some("crates"),
        "the selection must be recorded in the summary"
    );
    assert_eq!(
        summary["packages"][0].as_str(),
        Some("one"),
        "both selected crates must be indexed"
    );
    assert_eq!(summary["packages"][1].as_str(), Some("two"));

    // --all opts into the whole repository, including the dump's files in
    // the population scope (the code index still only extracts sources).
    let stdout = assert_ok(&[
        "index",
        "-i",
        &instance_path,
        "repository",
        "--all",
        &repo_path,
    ])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full rebuild: {stdout}"
    );
    assert!(
        stdout.contains("selected_paths=whole-repo"),
        "--all must select the whole repository: {stdout}"
    );
    Ok(())
}
