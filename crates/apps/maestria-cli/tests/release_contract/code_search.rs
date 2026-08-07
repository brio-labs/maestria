use maestria_cli::test_support::{TempDir, assert_init_ok, assert_ok, run, write_file};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = Command::new("git").current_dir(repo).args(args).status()?;
    if !status.success() {
        return Err(format!("git {args:?} failed in {}", repo.display()).into());
    }
    Ok(())
}

/// Build a git repo fixture that indexes cleanly with a module file, an impl
/// block (spanning chunk boundaries), and a newline-sensitive edit target.
fn make_repo(repo: &Path) -> Result<(), Box<dyn Error>> {
    write_file(
        repo,
        "Cargo.toml",
        r#"
[package]
name = "code_fixture"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        repo,
        "src/lib.rs",
        r#"
pub mod helpers;

/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 { a + b }

pub struct Widget {
    pub value: i32,
}

impl Widget {
    pub fn name(&self) -> i32 { self.value }
    pub fn calls(&self) -> i32 { self.name() }
}
"#,
    )?;
    write_file(repo, "src/helpers.rs", "pub fn helper() -> i32 { 1 }\n")?;
    run_git(repo, &["init", "--initial-branch", "main"])?;
    run_git(repo, &["config", "user.email", "ci@example.com"])?;
    run_git(repo, &["config", "user.name", "CI"])?;
    run_git(repo, &["add", "."])?;
    run_git(repo, &["commit", "-m", "fixture init"])?;
    Ok(())
}

/// Build a git repo fixture with two independent Cargo workspaces under one
/// repository root: a root workspace with member `crate_one` and an unrelated
/// nested workspace at `rust/tools` with member `tool_x`.
fn make_nested_repo(repo: &Path) -> Result<(), Box<dyn Error>> {
    write_file(
        repo,
        "Cargo.toml",
        r#"
[workspace]
members = ["crate_one"]

[workspace.package]
edition = "2024"
"#,
    )?;
    write_file(
        repo,
        "crate_one/Cargo.toml",
        r#"
[package]
name = "crate_one"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        repo,
        "crate_one/src/lib.rs",
        "pub fn root_add(a: i32, b: i32) -> i32 { a + b }\n",
    )?;
    write_file(
        repo,
        "rust/tools/Cargo.toml",
        r#"
[workspace]
members = ["tool_x"]

[workspace.package]
edition = "2024"
"#,
    )?;
    write_file(
        repo,
        "rust/tools/tool_x/Cargo.toml",
        r#"
[package]
name = "tool_x"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        repo,
        "rust/tools/tool_x/src/lib.rs",
        "pub fn nested_util() -> i32 { 42 }\n",
    )?;
    run_git(repo, &["init", "--initial-branch", "main"])?;
    run_git(repo, &["config", "user.email", "ci@example.com"])?;
    run_git(repo, &["config", "user.name", "CI"])?;
    run_git(repo, &["add", "."])?;
    run_git(repo, &["commit", "-m", "fixture init"])?;
    Ok(())
}

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

#[test]
fn repository_index_on_non_rust_repo_is_empty_and_fresh() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-nonrust-repo")?;
    let instance = TempDir::new("maestria-release-nonrust-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    write_file(repo.path(), "app.py", "def main():\n    return 42\n")?;
    write_file(repo.path(), "README.md", "# non-rust fixture\n")?;
    run_git(repo.path(), &["init", "--initial-branch", "main"])?;
    run_git(repo.path(), &["config", "user.email", "ci@example.com"])?;
    run_git(repo.path(), &["config", "user.name", "CI"])?;
    run_git(repo.path(), &["add", "."])?;
    run_git(repo.path(), &["commit", "-m", "fixture init"])?;
    assert_init_ok(&instance_path, &repo_path)?;

    // A repository without a Rust workspace indexes to a valid, fresh empty
    // index instead of failing.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    assert_eq!(summary["symbol_count"], serde_json::Value::from(0));

    let (matched, records) = search_code_symbol(&instance_path, "main")?;
    assert_eq!(
        matched, 0,
        "non-Rust repo must have no symbols: {records:?}"
    );

    // Unchanged repository is a no-op and stays searchable (no matches).
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(stdout.contains("mode=noop"), "expected noop: {stdout}");
    Ok(())
}

#[test]
fn repository_code_index_search_roundtrip() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-code-repo")?;
    let instance = TempDir::new("maestria-release-code-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_repo(repo.path())?;
    assert_init_ok(&instance_path, &repo_path)?;

    // First build is a full rebuild; the sources are registered as canonical
    // artifacts so code queries can authorize them.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "first repository index should be a full build: {stdout}"
    );

    // Symbols across files and inside impl blocks are all searchable.
    let (matched, records) = search_code_symbol(&instance_path, "add")?;
    assert!(
        matched >= 1,
        "symbol search for `add` found nothing: {records:?}"
    );
    assert!(
        records
            .iter()
            .any(|record| record.contains("src/lib.rs") && record.contains(":function:add:")),
        "expected the `add` function record: {records:?}"
    );
    let (matched, records) = search_code_symbol(&instance_path, "calls")?;
    assert!(
        matched >= 1 && records.iter().any(|record| record.contains("Widget")),
        "impl-block method `calls` not searchable: {records:?}"
    );
    let (matched, _) = search_code_symbol(&instance_path, "helper")?;
    assert!(matched >= 1, "module file symbol not searchable");

    // An edit re-parses only the affected file and keeps the index fresh.
    let lib = repo.path().join("src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("\npub fn subtracted() -> i32 { 3 }\n");
    fs::write(&lib, source)?;
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=incremental"),
        "edited repository should rebuild incrementally: {stdout}"
    );
    let (matched, records) = search_code_symbol(&instance_path, "subtracted")?;
    assert!(
        matched == 1 && records.iter().any(|record| record.contains("subtracted")),
        "new symbol not searchable after incremental rebuild: {records:?}"
    );

    // Unchanged repository is a no-op and stays searchable.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(stdout.contains("mode=noop"), "expected noop: {stdout}");
    let (matched, _) = search_code_symbol(&instance_path, "subtracted")?;
    assert_eq!(matched, 1, "noop rebuild must not break search");

    // Edits without reindexing still fail closed with the stale message.
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("\npub fn dirty_probe() -> i32 { 9 }\n");
    fs::write(&lib, source)?;
    let (code, stdout, stderr) = run(&[
        "search",
        "-i",
        &instance_path,
        "code",
        "symbol",
        "dirty_probe",
    ])?;
    assert_ne!(code, 0, "stale search unexpectedly succeeded: {stdout}");
    assert!(
        stderr.contains("repository code index is stale"),
        "expected stale freshness error: {stderr}"
    );
    Ok(())
}

#[test]
fn repository_code_index_covers_nested_workspaces() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-nested-repo")?;
    let instance = TempDir::new("maestria-release-nested-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_nested_repo(repo.path())?;
    assert_init_ok(&instance_path, &repo_path)?;

    // Both workspaces' packages are indexed into the single repository index.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "first repository index should be a full build: {stdout}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    let packages: Vec<&str> = summary["packages"]
        .as_array()
        .ok_or("missing packages")?
        .iter()
        .filter_map(|package| package.as_str())
        .collect();
    assert!(
        packages.contains(&"crate_one"),
        "root workspace package missing: {packages:?}"
    );
    assert!(
        packages.contains(&"tool_x"),
        "nested workspace package missing: {packages:?}"
    );
    assert_eq!(
        summary["workspace_warnings"].as_array().map_or(0, Vec::len),
        0,
        "healthy fixture must not warn: {stdout}"
    );

    // Symbols from both workspaces are searchable through the same index.
    let (matched, records) = search_code_symbol(&instance_path, "nested_util")?;
    assert!(
        matched >= 1
            && records
                .iter()
                .any(|record| record.contains("rust/tools/tool_x/src/lib.rs")),
        "nested workspace symbol not searchable: {records:?}"
    );
    let (matched, records) = search_code_symbol(&instance_path, "root_add")?;
    assert!(
        matched >= 1
            && records
                .iter()
                .any(|record| record.contains("crate_one/src/lib.rs")),
        "root workspace symbol not searchable: {records:?}"
    );

    // An edit inside the nested workspace rebuilds incrementally and the new
    // symbol is searchable.
    let nested = repo.path().join("rust/tools/tool_x/src/lib.rs");
    let mut source = fs::read_to_string(&nested)?;
    source.push_str("\npub fn nested_extra() -> i32 { 7 }\n");
    fs::write(&nested, source)?;
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=incremental"),
        "edited nested workspace should rebuild incrementally: {stdout}"
    );
    let (matched, records) = search_code_symbol(&instance_path, "nested_extra")?;
    assert!(
        matched == 1 && records.iter().any(|record| record.contains("nested_extra")),
        "new nested symbol not searchable after incremental rebuild: {records:?}"
    );
    Ok(())
}

#[test]
fn broken_nested_workspace_warns_on_stderr_and_indexes_healthy() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-broken-nested-repo")?;
    let instance = TempDir::new("maestria-release-broken-nested-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_nested_repo(repo.path())?;
    // A broken standalone nested workspace with a missing member.
    write_file(
        repo.path(),
        "rust/broken/Cargo.toml",
        "[workspace]\nmembers = [\"does_not_exist\"]\n",
    )?;
    run_git(repo.path(), &["add", "."])?;
    run_git(
        repo.path(),
        &["commit", "-m", "add broken nested workspace"],
    )?;
    assert_init_ok(&instance_path, &repo_path)?;

    // The index command succeeds, prints a `warning:` line naming the broken
    // workspace on stderr, and still indexes the healthy workspaces.
    let (code, stdout, stderr) = run(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert_eq!(code, 0, "index failed: {stderr}");
    assert!(
        stderr.contains("warning:") && stderr.contains("rust/broken"),
        "expected a warning naming the broken workspace: {stderr}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    assert!(
        summary["workspace_warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty()),
        "summary JSON must carry the warnings: {stdout}"
    );
    let (matched, _) = search_code_symbol(&instance_path, "nested_util")?;
    assert!(
        matched >= 1,
        "healthy nested workspace must stay searchable"
    );
    Ok(())
}
