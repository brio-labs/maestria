//! Retrieval-retirement service tests: boundary validation and
//! authentication requirements (ADR-0009). The durable happy path is
//! covered end-to-end by the CLI daemon tests.

use super::*;
use crate::FederationCredential;
use crate::test_support::TempDir;
use maestria_core::{InstanceLayout, InstanceManifest};
use std::path::PathBuf;

struct Fixture {
    _temp_dir: TempDir,
    layout: InstanceLayout,
}

fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
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
    })
}

fn context(layout: InstanceLayout) -> Result<ApiContext> {
    Ok(ApiContext {
        layout,
        token: "test-token".to_string(),
        socket_path: PathBuf::new(),
        runtime: None,
        realm_id: maestria_test_support::realm_id(10)?,
    })
}

#[tokio::test]
async fn retire_rejects_zero_boundary_before_touching_runtime() -> Result<()> {
    let fixture = fixture()?;
    let context = context(fixture.layout)?;
    let result = retire(&context, &RequestPrincipal::Instance, 0, "audit".into()).await;
    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn retire_rejects_empty_reason_before_touching_runtime() -> Result<()> {
    let fixture = fixture()?;
    let context = context(fixture.layout)?;
    let result = retire(&context, &RequestPrincipal::Instance, 5, "   ".into()).await;
    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn retire_requires_instance_authentication() -> Result<()> {
    let fixture = fixture()?;
    let context = context(fixture.layout)?;
    let result = retire(
        &context,
        &RequestPrincipal::Federation {
            consumer_realm: maestria_test_support::realm_id(11)?,
            credential: FederationCredential::try_from("a".repeat(64))?,
        },
        5,
        "audit".into(),
    )
    .await;
    assert!(result.is_err());
    Ok(())
}
