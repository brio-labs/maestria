//! Index choice service tests: candidates, selection round-trip, and run
//! validation.

use super::*;
use maestria_core::{InstanceLayout, InstanceManifest};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-index-services-test-{}-{id}",
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
    read_root: PathBuf,
}

fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
    let read_root = root.join("workspace");
    std::fs::create_dir_all(read_root.join("docs"))?;
    std::fs::create_dir_all(read_root.join("code"))?;
    std::fs::create_dir_all(read_root.join("dump"))?;
    std::fs::write(read_root.join("docs/a.md"), "# doc\n")?;
    std::fs::write(read_root.join("code/main.rs"), "fn main() {}\n")?;
    for index in 0..200 {
        std::fs::write(read_root.join(format!("dump/f{index}.json")), "{\"k\":1}")?;
    }
    let layout = InstanceLayout::for_root(root);
    std::fs::create_dir_all(&layout.system_dir)?;
    if let Some(parent) = layout.database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = InstanceManifest::default_for_root(
        layout.root.clone(),
        maestria_test_support::realm_id(10)?,
    );
    std::fs::write(&layout.manifest_path, manifest.encode())?;
    Ok(Fixture {
        _temp_dir: temp_dir,
        layout,
        read_root,
    })
}

fn status_context(layout: InstanceLayout) -> Result<ApiContext> {
    Ok(ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path: PathBuf::new(),
        runtime: None,
        realm_id: maestria_test_support::realm_id(10)?,
    })
}

#[tokio::test]
async fn candidates_classifies_fixture_tree() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let response = candidates(&context, fixture.read_root.display().to_string()).await?;
    assert_eq!(response.root, fixture.read_root.display().to_string());
    assert!(!response.home_root);
    assert_eq!(response.tree.file_count, 202);

    let docs = response
        .tree
        .children
        .iter()
        .find(|child| child.path.ends_with("docs"))
        .ok_or_else(|| anyhow!("docs child missing"))?;
    assert_eq!(docs.class, maestria_index_selection::Class::Recommended);
    assert_eq!(docs.file_count, 1);

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
        "maestria-index-services-outside-{}-1",
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
        root: fixture.read_root.clone(),
        includes: vec![
            fixture.read_root.join("docs"),
            fixture.read_root.join("code"),
        ],
        policies: std::collections::BTreeMap::new(),
    };
    selection_save(&context, profile.clone()).await?;

    let after = selection_get(&context).await?;
    assert_eq!(after.profile, Some(profile));
    Ok(())
}

#[tokio::test]
async fn selection_save_rejects_outside_include() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let profile = maestria_index_selection::IndexSelectionProfile {
        root: fixture.read_root.clone(),
        includes: vec![fixture.read_root.join("..").join("outside.md")],
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
async fn run_requires_live_runtime() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let error = match run(
        &context,
        fixture.read_root.display().to_string(),
        vec![fixture.read_root.display().to_string()],
        std::collections::BTreeMap::new(),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => return Err(anyhow!("run without a runtime must fail")),
    };
    assert!(
        error.to_string().contains("requires the live runtime"),
        "unexpected runtime error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn run_rejects_out_of_scope_root() -> Result<()> {
    let fixture = fixture()?;
    let context = status_context(fixture.layout)?;
    let error = match run(
        &context,
        "/tmp/definitely-outside-maestria-scope".to_string(),
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
        .read_root
        .join("..")
        .join("outside.md")
        .display()
        .to_string();
    let error = match run(
        &context,
        fixture.read_root.display().to_string(),
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
