//! Instance write-lock liveness tests (Linux: /proc start-tick
//! discrimination).

use super::*;
use crate::test_support::TempDir;
use maestria_core::{InstanceLayout, InstanceManifest};

struct Fixture {
    _temp_dir: TempDir,
    layout: InstanceLayout,
}

fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let layout = InstanceLayout::for_root(temp_dir.path().to_path_buf());
    fs::create_dir_all(&layout.system_dir)?;
    let manifest = InstanceManifest::default_for_root(
        layout.root.clone(),
        maestria_test_support::realm_id(10)?,
    );
    fs::write(&layout.manifest_path, manifest.encode())?;
    Ok(Fixture {
        _temp_dir: temp_dir,
        layout,
    })
}

fn write_lock(layout: &InstanceLayout, token: &str) -> Result<PathBuf> {
    let path = layout.system_dir.join("instance-write.lock");
    fs::write(&path, format!("{token}\n"))
        .map_err(|error| anyhow!("write lock fixture: {error}"))?;
    Ok(path)
}

#[test]
fn lock_with_reused_pid_is_recognized_as_stale() -> Result<()> {
    let fixture = fixture()?;
    let ticks = process_start_ticks().ok_or_else(|| anyhow!("linux test requires /proc"))?;
    // Simulate a dead holder whose pid was reused: same pid as this
    // process, different start ticks.
    write_lock(
        &fixture.layout,
        &format!("{}:t{}", std::process::id(), ticks + 12_345),
    )?;
    let acquired = try_acquire(&fixture.layout)?;
    assert!(
        acquired.is_some(),
        "a reused-pid lock must be quarantined instead of blocking acquisition"
    );
    Ok(())
}

#[test]
fn lock_with_matching_start_ticks_is_live() -> Result<()> {
    let fixture = fixture()?;
    let ticks = process_start_ticks().ok_or_else(|| anyhow!("linux test requires /proc"))?;
    write_lock(&fixture.layout, &format!("{}:t{ticks}", std::process::id()))?;
    let acquired = try_acquire(&fixture.layout)?;
    assert!(
        acquired.is_none(),
        "a lock held by a live process incarnation must not be stolen"
    );
    Ok(())
}

#[test]
fn lock_without_discriminator_falls_back_to_pid_liveness() -> Result<()> {
    let fixture = fixture()?;
    // Legacy wall-clock nonce token: pid alive -> treated as live owner.
    write_lock(&fixture.layout, &format!("{}:1", std::process::id()))?;
    let acquired = try_acquire(&fixture.layout)?;
    assert!(acquired.is_none(), "legacy token with live pid stays live");
    Ok(())
}
