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

/// Run `search code doc <pattern>` and return (matched, record ids).
fn search_code_doc(
    instance_path: &str,
    pattern: &str,
) -> Result<(usize, Vec<String>), Box<dyn Error>> {
    let stdout = assert_ok(&["search", "-i", instance_path, "code", "doc", pattern])?;
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

/// Run `search code markers <kind>` and return (matched, record ids).
fn search_code_markers(
    instance_path: &str,
    kind: &str,
) -> Result<(usize, Vec<String>), Box<dyn Error>> {
    let stdout = assert_ok(&["search", "-i", instance_path, "code", "markers", kind])?;
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

/// Exercise `search code doc` and `search code markers` end to end: doc
/// records, deterministic marker counts, and the invalid-kind parse error.
fn assert_doc_and_marker_search(instance_path: &str) -> Result<(), Box<dyn Error>> {
    let (matched, records) = search_code_doc(instance_path, "Adds two numbers")?;
    assert!(
        matched >= 1
            && records
                .iter()
                .any(|record| record.contains(":function:add:")),
        "doc search for `add` found nothing: {records:?}"
    );
    let (matched, _) = search_code_markers(instance_path, "todo")?;
    assert_eq!(matched, 0, "fixture has no todo markers yet: {matched}");
    let (code, _stdout, stderr) =
        run(&["search", "-i", instance_path, "code", "markers", "bogus"])?;
    assert_ne!(code, 0, "invalid marker kind must fail");
    assert!(
        stderr.contains("invalid marker kind"),
        "expected marker parse error: {stderr}"
    );
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

    // Doc-comment and marker search go through the same run_search path.
    assert_doc_and_marker_search(&instance_path)?;

    // An edit re-parses only the affected file and keeps the index fresh.
    let lib = repo.path().join("src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str(
        "\n/// Subtracted docs.\npub fn subtracted() -> i32 {\n    // todo: subtract later\n    3\n}\n",
    );
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
    let (matched, records) = search_code_doc(&instance_path, "Subtracted docs")?;
    assert!(
        matched == 1 && records.iter().any(|record| record.contains("subtracted")),
        "doc text not searchable after incremental rebuild: {records:?}"
    );
    let (matched, records) = search_code_markers(&instance_path, "todo")?;
    assert!(
        matched == 1 && records.iter().any(|record| record.contains("subtracted")),
        "todo marker not searchable after incremental rebuild: {records:?}"
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
    // Doc-comment search shares the same freshness gate.
    let (code, stdout, stderr) = run(&[
        "search",
        "-i",
        &instance_path,
        "code",
        "doc",
        "Subtracted docs",
    ])?;
    assert_ne!(code, 0, "stale doc search unexpectedly succeeded: {stdout}");
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

#[test]
fn repository_code_changed_query_flow() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-changed-repo")?;
    let instance = TempDir::new("maestria-release-changed-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_repo(repo.path())?;
    assert_init_ok(&instance_path, &repo_path)?;

    // A clean full build reports an empty changed section.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    assert!(
        stdout.contains("changed_files=0 changed_symbols=0"),
        "clean full build must report an empty changed section: {stdout}"
    );

    // Commit an edit and rebuild: the summary gains the edited file and its
    // symbols through the baseline..HEAD diff.
    let lib = repo.path().join("src/lib.rs");
    let mut source = fs::read_to_string(&lib)?;
    source.push_str("\npub fn changed_fn() -> i32 { 4 }\n");
    fs::write(&lib, source)?;
    run_git(repo.path(), &["add", "."])?;
    run_git(repo.path(), &["commit", "-m", "edit"])?;
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=incremental"),
        "edited repository should rebuild incrementally: {stdout}"
    );
    assert!(
        stdout.contains("changed_files=1"),
        "committed edit must be in the changed section: {stdout}"
    );

    // `search code changed` uses the persisted delta: the edited file's
    // symbols exactly.
    let stdout = assert_ok(&["search", "-i", &instance_path, "code", "changed"])?;
    let value: serde_json::Value = serde_json::from_str(&stdout)?;
    let matched = value["summary"]["matched"]
        .as_u64()
        .ok_or("missing matched")?;
    assert!(matched >= 1, "changed query found nothing: {stdout}");
    let records = value["records"].as_array().ok_or("missing records")?;
    assert!(
        records
            .iter()
            .all(|record| record["provenance"]["file_path"] == "src/lib.rs"),
        "changed query must only return edited-file symbols: {stdout}"
    );
    assert!(
        records.iter().any(|record| record["record_id"]
            .as_str()
            .is_some_and(|id| id.contains(":function:changed_fn:"))),
        "changed query must include the newly committed symbol: {stdout}"
    );

    // `search code changed --since HEAD~1` resolves the same delta live
    // (git diff plus the current dirty set) and matches the same symbols.
    let stdout = assert_ok(&[
        "search",
        "-i",
        &instance_path,
        "code",
        "changed",
        "--since",
        "HEAD~1",
    ])?;
    let value: serde_json::Value = serde_json::from_str(&stdout)?;
    let matched = value["summary"]["matched"]
        .as_u64()
        .ok_or("missing matched")?;
    assert!(matched >= 1, "live changed query found nothing: {stdout}");
    let records = value["records"].as_array().ok_or("missing records")?;
    assert!(
        records
            .iter()
            .all(|record| record["provenance"]["file_path"] == "src/lib.rs"),
        "live changed query must only return edited-file symbols: {stdout}"
    );

    // Garbage --since values fail before any git call.
    let (code, stdout, stderr) = run(&[
        "search",
        "-i",
        &instance_path,
        "code",
        "changed",
        "--since",
        "not-a-commit",
    ])?;
    assert_ne!(code, 0, "garbage --since unexpectedly succeeded: {stdout}");
    assert!(
        stderr.contains("invalid commit reference"),
        "expected invalid commit reference error: {stderr}"
    );
    Ok(())
}

/// Build a git repo with a PEP 621 manifest and a small `src/` package.
fn make_python_repo(repo: &Path) -> Result<(), Box<dyn Error>> {
    write_file(
        repo,
        "pyproject.toml",
        "[project]\nname = \"wishlist\"\nversion = \"0.1.0\"\n",
    )?;
    write_file(
        repo,
        "src/wishlist/__init__.py",
        "from wishlist.items import Item\n",
    )?;
    write_file(
        repo,
        "src/wishlist/items.py",
        "class Item:\n    def __init__(self, name):\n        self.name = name\n\n    def total(self):\n        return 1\n",
    )?;
    run_git(repo, &["init", "--initial-branch", "main"])?;
    run_git(repo, &["config", "user.email", "ci@example.com"])?;
    run_git(repo, &["config", "user.name", "CI"])?;
    run_git(repo, &["add", "."])?;
    run_git(repo, &["commit", "-m", "fixture init"])?;
    Ok(())
}

#[test]
fn python_repository_code_index_search_roundtrip() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-python-repo")?;
    let instance = TempDir::new("maestria-release-python-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_python_repo(repo.path())?;
    assert_init_ok(&instance_path, &repo_path)?;

    // Python repositories index with real symbols and searchable records.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    let symbol_count = summary["symbol_count"]
        .as_u64()
        .ok_or("missing symbol_count")?;
    assert!(symbol_count >= 4);

    let (matched, records) = search_code_symbol(&instance_path, "Item")?;
    assert!(
        matched >= 1 && records.iter().any(|record| record.contains(":class:")),
        "python class not searchable: {records:?}"
    );
    let (matched, _) = search_code_symbol(&instance_path, "total")?;
    assert!(matched >= 1, "python method not searchable");

    // An edit rebuilds incrementally and stays searchable.
    let items = repo.path().join("src/wishlist/items.py");
    let mut source = fs::read_to_string(&items)?;
    source.push_str("\n\ndef make_item(name):\n    return Item(name)\n");
    fs::write(&items, source)?;
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=incremental"),
        "expected incremental: {stdout}"
    );
    let (matched, _) = search_code_symbol(&instance_path, "make_item")?;
    assert_eq!(
        matched, 1,
        "new python symbol not searchable after incremental rebuild"
    );

    // Unchanged repository is a no-op.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(stdout.contains("mode=noop"), "expected noop: {stdout}");
    Ok(())
}

/// Build a git repo with a `package.json` and a small `src/` tree with a
/// JSX component and an import chain.
fn make_web_repo(repo: &Path) -> Result<(), Box<dyn Error>> {
    write_file(
        repo,
        "package.json",
        "{\n  \"name\": \"ui-kit\",\n  \"version\": \"0.1.0\",\n  \"main\": \"src/index.ts\"\n}\n",
    )?;
    write_file(
        repo,
        "src/index.ts",
        "import { Button } from \"./components/Button\";\n\nexport function createDefaultItem() {\n  return Button({ label: \"Go\" });\n}\n",
    )?;
    write_file(
        repo,
        "src/components/Button.tsx",
        "export function Button({ label }: { label: string }) {\n  return <button>{label}</button>;\n}\n",
    )?;
    run_git(repo, &["init", "--initial-branch", "main"])?;
    run_git(repo, &["config", "user.email", "ci@example.com"])?;
    run_git(repo, &["config", "user.name", "CI"])?;
    run_git(repo, &["add", "."])?;
    run_git(repo, &["commit", "-m", "fixture init"])?;
    Ok(())
}

#[test]
fn web_repository_code_index_search_roundtrip() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-web-repo")?;
    let instance = TempDir::new("maestria-release-web-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    make_web_repo(repo.path())?;
    assert_init_ok(&instance_path, &repo_path)?;

    // Web repositories index with real symbols and searchable records.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    let summary_start = stdout.find('{').ok_or("missing summary JSON")?;
    let summary: serde_json::Value = serde_json::from_str(&stdout[summary_start..])?;
    let symbol_count = summary["symbol_count"]
        .as_u64()
        .ok_or("missing symbol_count")?;
    assert!(symbol_count >= 4);

    let (matched, records) = search_code_symbol(&instance_path, "Button")?;
    assert!(
        matched >= 1
            && records
                .iter()
                .any(|record| record.contains(":function:") && record.contains("Button.tsx")),
        "web component not searchable: {records:?}"
    );
    let (matched, _) = search_code_symbol(&instance_path, "createDefaultItem")?;
    assert!(matched >= 1, "web function not searchable");

    // An edit rebuilds incrementally and stays searchable.
    let index_file = repo.path().join("src/index.ts");
    let mut source = fs::read_to_string(&index_file)?;
    source.push_str("\nexport function catalogCount(): number {\n  return 3;\n}\n");
    fs::write(&index_file, source)?;
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(
        stdout.contains("mode=incremental"),
        "expected incremental: {stdout}"
    );
    let (matched, _) = search_code_symbol(&instance_path, "catalogCount")?;
    assert_eq!(
        matched, 1,
        "new web symbol not searchable after incremental rebuild"
    );

    // Unchanged repository is a no-op.
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(stdout.contains("mode=noop"), "expected noop: {stdout}");
    Ok(())
}

#[test]
fn repository_index_with_empty_source_files_succeeds() -> Result<(), Box<dyn Error>> {
    let repo = TempDir::new("maestria-release-empty-repo")?;
    let instance = TempDir::new("maestria-release-empty-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let repo_path = repo.path().to_string_lossy().into_owned();
    write_file(
        repo.path(),
        "pyproject.toml",
        "[project]\nname = \"empty_fixture\"\nversion = \"0.1.0\"\n",
    )?;
    // An empty package file still carries a module symbol (the python
    // extractor emits one per file), but it parses to zero chunks;
    // registration must skip it instead of waiting forever for an Indexed
    // state that never arrives.
    write_file(repo.path(), "src/empty_pkg/__init__.py", "")?;
    write_file(
        repo.path(),
        "src/empty_pkg/mod.py",
        "def probe():\n    return 1\n",
    )?;
    run_git(repo.path(), &["init", "-q"])?;
    run_git(repo.path(), &["config", "user.email", "ci@example.com"])?;
    run_git(repo.path(), &["config", "user.name", "CI"])?;
    run_git(repo.path(), &["add", "."])?;
    run_git(repo.path(), &["commit", "-m", "add empty source"])?;
    assert_init_ok(&instance_path, &repo_path)?;

    let (code, stdout, stderr) =
        run(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert_eq!(code, 0, "index failed: {stderr}");
    assert!(
        stdout.contains("mode=full"),
        "expected full build: {stdout}"
    );
    assert!(
        stderr.contains("skipped 1 repository source(s)"),
        "expected the empty source to be skipped: {stderr}"
    );

    // Non-empty symbols remain searchable, and a second run is a no-op.
    let (matched, _) = search_code_symbol(&instance_path, "probe")?;
    assert!(matched >= 1, "non-empty symbols must stay searchable");
    let stdout = assert_ok(&["index", "-i", &instance_path, "repository", &repo_path])?;
    assert!(stdout.contains("mode=noop"), "expected noop: {stdout}");
    Ok(())
}
