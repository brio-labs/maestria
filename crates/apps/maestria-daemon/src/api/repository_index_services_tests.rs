//! Repository index service tests: scope and containment rejection,
//! candidates shape, selection round-trip, run happy path, and status.

use super::*;
use maestria_core::InstanceLayout;
use maestria_domain::RealmId;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-repository-index-services-test-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _temp_dir: TempDir,
    layout: InstanceLayout,
    repo: PathBuf,
}

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

/// A two-crate git repository fixture: `crates/one` + `crates/two` Rust
/// packages and a generated dump, committed.
fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("crates/one/src"))?;
    std::fs::create_dir_all(repo.join("crates/two/src"))?;
    std::fs::create_dir_all(repo.join("dump"))?;
    std::fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/one\", \"crates/two\"]\n",
    )?;
    std::fs::write(
        repo.join("crates/one/Cargo.toml"),
        "[package]\nname = \"one\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\n\
         path = \"src/lib.rs\"\n",
    )?;
    std::fs::write(
        repo.join("crates/one/src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )?;
    std::fs::write(
        repo.join("crates/two/Cargo.toml"),
        "[package]\nname = \"two\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\n\
         path = \"src/lib.rs\"\n",
    )?;
    std::fs::write(
        repo.join("crates/two/src/lib.rs"),
        "pub fn two() -> i32 { 2 }\n",
    )?;
    for index in 0..200 {
        std::fs::write(repo.join(format!("dump/f{index:03}.json")), "{\"k\":1}")?;
    }
    run_git(&repo, &["init", "--initial-branch", "main"])?;
    run_git(&repo, &["config", "user.email", "ci@example.com"])?;
    run_git(&repo, &["config", "user.name", "CI"])?;
    run_git(&repo, &["add", "."])?;
    run_git(&repo, &["commit", "-m", "fixture init"])?;

    // The full instance layout (system dir, database, indexes) is created
    // by `prepare_instance`, which the runtime lifecycle requires.
    let layout = crate::prepare_instance(root)?;
    Ok(Fixture {
        _temp_dir: temp_dir,
        layout,
        repo,
    })
}

fn status_context(layout: InstanceLayout) -> Result<ApiContext> {
    Ok(ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path: PathBuf::new(),
        runtime: None,
        realm_id: RealmId::try_from("a".repeat(64))?,
    })
}

#[tokio::test]
async fn candidates_classifies_repository_fixture_tree() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let response = candidates(&context, fixture.repo.display().to_string()).await?;
    assert_eq!(response.root, fixture.repo.display().to_string());
    // 2 crate sources + 2 manifests + 200 dump files + 1 root manifest.
    assert_eq!(response.tree.file_count, 205);
    assert_eq!(
        response.tree.class,
        maestria_index_selection::Class::Recommended
    );

    let crates = response
        .tree
        .children
        .iter()
        .find(|child| child.path.ends_with("crates"))
        .ok_or_else(|| anyhow!("crates child missing"))?;
    assert_eq!(crates.class, maestria_index_selection::Class::Recommended);
    assert_eq!(crates.file_count, 4);

    let dump = response
        .tree
        .children
        .iter()
        .find(|child| child.path.ends_with("dump"))
        .ok_or_else(|| anyhow!("dump child missing"))?;
    assert_eq!(dump.class, maestria_index_selection::Class::Noise);
    assert_eq!(dump.file_count, 200);
    Ok(())
}

#[tokio::test]
async fn candidates_rejects_out_of_scope_root() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let outside = std::env::temp_dir().join(format!(
        "maestria-repository-index-services-outside-{}-1",
        std::process::id()
    ));
    std::fs::create_dir_all(&outside)?;
    let error = match candidates(&context, outside.display().to_string()).await {
        Err(error) => error,
        Ok(_) => return Err(anyhow!("out-of-scope root must be rejected")),
    };
    assert!(
        error
            .to_string()
            .contains("outside the instance read scope"),
        "unexpected scope error: {error}"
    );
    std::fs::remove_dir_all(&outside)?;
    Ok(())
}

#[tokio::test]
async fn selection_save_and_get_round_trip() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout.clone())?;
    let before = selection_get(&context).await?;
    assert!(before.profile.is_none());

    let profile = maestria_index_selection::IndexSelectionProfile {
        root: fixture.repo.clone(),
        includes: vec![PathBuf::from("crates/one")],
        policies: std::collections::BTreeMap::from([(
            PathBuf::from("crates/one"),
            IndexPolicy {
                max_file_bytes: 1024,
                ..IndexPolicy::everything()
            },
        )]),
    };
    selection_save(&context, profile.clone()).await?;

    // The profile round-trips with includes/policies normalized to
    // repository-relative paths.
    let after = selection_get(&context).await?;
    assert_eq!(after.profile, Some(profile));
    Ok(())
}

#[tokio::test]
async fn selection_save_rejects_outside_include() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let profile = maestria_index_selection::IndexSelectionProfile {
        root: fixture.repo.clone(),
        includes: vec![fixture.repo.join("..").join("outside.md")],
        policies: std::collections::BTreeMap::new(),
    };
    let error = match selection_save(&context, profile).await {
        Err(error) => error,
        Ok(()) => return Err(anyhow!("out-of-root include must be rejected")),
    };
    assert!(
        error.to_string().contains("selection path"),
        "unexpected include error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn run_rejects_out_of_scope_root() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let error = match run(
        &context,
        "/tmp/definitely-outside-maestria-repository-scope".to_string(),
        vec![],
        std::collections::BTreeMap::new(),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => return Err(anyhow!("out-of-scope run must fail")),
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
async fn run_rejects_out_of_root_include() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let escaping = fixture
        .repo
        .join("..")
        .join("outside.md")
        .display()
        .to_string();
    let error = match run(
        &context,
        fixture.repo.display().to_string(),
        vec![escaping],
        std::collections::BTreeMap::new(),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => return Err(anyhow!("out-of-root include must be rejected")),
    };
    assert!(
        error.to_string().contains("escapes the selection root"),
        "unexpected include error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn run_builds_selected_index_and_status_reports_present() -> Result<()> {
    let fixture = fixture()?;
    let layout = fixture.layout.clone();
    let lifecycle = crate::InstanceLifecycle::start(
        layout.clone(),
        maestria_governance::AutonomyProfile::TrustedWorkspace,
    )
    .await?;
    let runtime = lifecycle.runtime_handle();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let runtime_task = tokio::spawn(lifecycle.run_until_shutdown(shutdown.clone()));
    let context = ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path: PathBuf::new(),
        runtime: Some(runtime),
        realm_id: RealmId::try_from("a".repeat(64))?,
    };

    let includes = vec![fixture.repo.join("crates/one").display().to_string()];
    let response = run(
        &context,
        fixture.repo.display().to_string(),
        includes,
        std::collections::BTreeMap::new(),
    )
    .await?;
    assert_eq!(response.mode, "full");
    assert_eq!(
        response.summary.selected_paths,
        vec!["crates/one".to_string()]
    );
    assert_eq!(response.summary.packages, vec!["one".to_string()]);
    assert!(response.registered >= 1, "sources must be registered");

    // Status reports the persisted index with a current freshness.
    let status = status(&context, fixture.repo.display().to_string()).await?;
    assert!(status.present);
    let summary = status
        .summary
        .ok_or_else(|| anyhow!("present index must carry a summary"))?;
    assert_eq!(summary.selected_paths, vec!["crates/one".to_string()]);
    assert!(
        matches!(
            status.freshness,
            Some(maestria_code_intel::RepositoryFreshness::Current { .. })
        ),
        "freshly indexed repository must be current"
    );

    shutdown.cancel();
    runtime_task
        .await
        .map_err(|join| anyhow!("runtime task join failed: {join}"))?
        .map_err(|error| anyhow!("clean runtime shutdown failed: {error:#}"))?;
    Ok(())
}
