//! Repository index browse service tests: lazy expansion and file listing.

use super::*;
use crate::test_support::TempDir;
use maestria_core::InstanceLayout;
use maestria_domain::RealmId;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .map_err(|error| anyhow!("spawn git {args:?}: {error}"))?;
    if !status.success() {
        return Err(anyhow!("git {args:?} failed in {}", repo.display()));
    }
    Ok(())
}

struct Fixture {
    _temp_dir: TempDir,
    layout: InstanceLayout,
    repo: PathBuf,
}

fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("crates/one/src"))?;
    std::fs::create_dir_all(repo.join("crates/two"))?;
    std::fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/one\"]\n",
    )?;
    std::fs::write(
        repo.join("crates/one/src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )?;
    std::fs::write(repo.join("crates/two/notes.md"), "# notes\n")?;
    std::fs::write(repo.join("README.md"), "# repo\n")?;
    run_git(&repo, &["init", "--initial-branch", "main"])?;
    run_git(&repo, &["config", "user.email", "ci@example.com"])?;
    run_git(&repo, &["config", "user.name", "CI"])?;
    run_git(&repo, &["add", "."])?;
    run_git(&repo, &["commit", "-m", "fixture init"])?;
    let layout = crate::prepare_instance(root)?;
    Ok(Fixture {
        _temp_dir: temp_dir,
        layout,
        repo,
    })
}

fn context(layout: InstanceLayout) -> Result<ApiContext> {
    Ok(ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path: PathBuf::new(),
        runtime: None,
        realm_id: RealmId::try_from("a".repeat(64))?,
    })
}

#[tokio::test]
async fn children_expands_one_level_with_classification() -> Result<()> {
    let fixture = fixture()?;
    let ctx = context(fixture.layout)?;
    let response = children(
        &ctx,
        fixture.repo.display().to_string(),
        "crates".to_string(),
    )
    .await?;
    assert_eq!(response.path, "crates");
    assert_eq!(response.children.len(), 2);
    let one = response
        .children
        .iter()
        .find(|child| child.path.ends_with("crates/one"))
        .ok_or_else(|| anyhow!("crates/one missing"))?;
    assert_eq!(one.class, maestria_index_selection::Class::Recommended);
    assert_eq!(one.file_count, 1);
    assert!(
        one.children.is_empty(),
        "children must be fetched on demand"
    );
    let two = response
        .children
        .iter()
        .find(|child| child.path.ends_with("crates/two"))
        .ok_or_else(|| anyhow!("crates/two missing"))?;
    assert_eq!(two.class, maestria_index_selection::Class::Recommended);
    Ok(())
}

#[tokio::test]
async fn children_excludes_direct_files() -> Result<()> {
    let fixture = fixture()?;
    let ctx = context(fixture.layout)?;
    // The fixture root has direct files (Cargo.toml, README.md): they must
    // not appear as expandable subdirectory rows.
    let response = children(&ctx, fixture.repo.display().to_string(), "".to_string()).await?;
    assert_eq!(
        response.children.len(),
        1,
        "only crates is a real subdirectory"
    );
    assert!(response.children[0].path.ends_with("crates"));
    // A source directory with only files expands to no subdirectories.
    let response = children(
        &ctx,
        fixture.repo.display().to_string(),
        "crates/one/src".to_string(),
    )
    .await?;
    assert!(
        response.children.is_empty(),
        "files must not be returned as children"
    );
    Ok(())
}

#[tokio::test]
async fn children_rejects_out_of_scope_root() -> Result<()> {
    let fixture = fixture()?;
    let ctx = context(fixture.layout)?;
    let error = match children(
        &ctx,
        "/tmp/definitely-outside-maestria-scope".to_string(),
        "crates".to_string(),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => return Err(anyhow!("out-of-scope root must be rejected")),
    };
    assert!(
        error
            .to_string()
            .contains("outside the instance read scope"),
        "unexpected scope error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn children_rejects_escaping_path() -> Result<()> {
    let fixture = fixture()?;
    let ctx = context(fixture.layout)?;
    let error = match children(
        &ctx,
        fixture.repo.display().to_string(),
        "../outside".to_string(),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => return Err(anyhow!("escaping path must be rejected")),
    };
    assert!(
        error.to_string().contains("repository-relative"),
        "unexpected path error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn files_lists_direct_files_with_kinds() -> Result<()> {
    let fixture = fixture()?;
    let ctx = context(fixture.layout)?;
    let response = files(&ctx, fixture.repo.display().to_string(), "".to_string()).await?;
    assert!(!response.truncated);
    let kinds: Vec<(String, String)> = response
        .files
        .iter()
        .map(|file| (file.path.clone(), file.kind.clone()))
        .collect();
    assert!(kinds.contains(&("Cargo.toml".to_string(), "manifest".to_string())));
    assert!(kinds.contains(&("README.md".to_string(), "doc".to_string())));
    // Nested files are not listed at the root.
    assert!(
        !kinds.iter().any(|(path, _)| path.starts_with("crates/")),
        "only direct files may be listed: {kinds:?}"
    );

    let response = files(
        &ctx,
        fixture.repo.display().to_string(),
        "crates/one/src".to_string(),
    )
    .await?;
    assert_eq!(response.files.len(), 1);
    assert_eq!(response.files[0].path, "crates/one/src/lib.rs");
    assert_eq!(response.files[0].kind, "code");
    Ok(())
}

#[tokio::test]
async fn files_truncate_at_the_wire_cap() -> Result<()> {
    let fixture = fixture()?;
    let ctx = context(fixture.layout)?;
    std::fs::create_dir_all(fixture.repo.join("many"))?;
    for index in 0..(MAX_DIRECT_FILES + 20) {
        std::fs::write(fixture.repo.join(format!("many/f{index:03}.md")), "# x\n")?;
    }
    let response = files(&ctx, fixture.repo.display().to_string(), "many".to_string()).await?;
    assert!(response.truncated, "the cap must be reported");
    assert_eq!(response.files.len(), MAX_DIRECT_FILES);
    Ok(())
}
