use maestria_code_intel::*;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn run_git(root: &Path, args: &[&str], operation: &str) -> Result<(), Box<dyn Error>> {
    let status = Command::new("git").current_dir(root).args(args).status()?;
    if !status.success() {
        return Err(format!("{operation} failed in {}: exit {status}", root.display()).into());
    }
    Ok(())
}

fn init_git(root: &Path) -> Result<(), Box<dyn Error>> {
    run_git(root, &["init", "--initial-branch", "main"], "git init")?;
    run_git(
        root,
        &["config", "user.email", "ci@example.com"],
        "git config user.email",
    )?;
    run_git(root, &["config", "user.name", "CI"], "git config user.name")?;
    run_git(root, &["add", "."], "git add")?;
    run_git(root, &["commit", "-m", "fixture init"], "git commit")
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn make_workspace() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let root = tempdir()?;
    write_file(
        &root.path().join("Cargo.toml"),
        r#"
[package]
name = "fixture"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )?;
    write_file(
        &root.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )?;
    init_git(root.path())?;
    Ok(root)
}

#[test]
fn freshness_reports_current_for_fresh_index() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;

    match index.freshness()? {
        RepositoryFreshness::Current { indexed, current } => {
            assert_eq!(indexed, current);
            assert_eq!(indexed.commit_sha, index.summary.commit_sha);
            assert_eq!(indexed.worktree_identity, index.summary.worktree_identity);
        }
        _ => {
            return Err("expected fresh index to be current".into());
        }
    }

    Ok(())
}

#[test]
fn freshness_detects_stale_worktree_changes() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;

    write_file(
        &tmp.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
    )?;

    match index.freshness()? {
        RepositoryFreshness::Stale { indexed, current } => {
            assert_eq!(indexed.commit_sha, current.commit_sha);
            assert_eq!(indexed.commit_sha, index.summary.commit_sha);
            assert_ne!(indexed.worktree_identity, current.worktree_identity);
        }
        _ => {
            return Err("expected modified source to produce stale freshness".into());
        }
    }

    Ok(())
}

#[test]
fn freshness_classifies_deleted_tracked_source_as_stale() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let index = RepositoryCodeIndex::build(tmp.path(), "g1")?;

    fs::remove_file(tmp.path().join("src/lib.rs"))?;

    match index.freshness()? {
        RepositoryFreshness::Stale { indexed, current } => {
            assert_eq!(indexed.commit_sha, current.commit_sha);
            assert_ne!(indexed.worktree_identity, current.worktree_identity);
        }
        _ => {
            return Err("expected deleted tracked source to produce stale freshness".into());
        }
    }

    Ok(())
}

#[test]
fn freshness_is_unaffected_by_excluded_paths() -> Result<(), Box<dyn Error>> {
    let tmp = make_workspace()?;
    let exclusions = vec!["ignored".to_string()];
    let index = RepositoryCodeIndex::build_with_exclusions(tmp.path(), "g1", &exclusions)?;

    let initial = index.freshness()?;

    write_file(
        &tmp.path().join("ignored/generated.rs"),
        "pub fn generated() -> i32 { 1 }\n",
    )?;

    let after_create = index.freshness()?;
    assert_eq!(after_create, initial);

    write_file(
        &tmp.path().join("ignored/generated.rs"),
        "pub fn generated() -> i32 { 2 }\n",
    )?;

    let after_modify = index.freshness()?;
    assert_eq!(after_modify, initial);

    Ok(())
}
