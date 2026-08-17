use super::*;
use std::{env, fs, process};

#[test]
fn state_persistence_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let dir = env::temp_dir().join(format!("maestria-watcher-state-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;

    let layout = InstanceLayout::for_root(dir.clone());
    fs::create_dir_all(&layout.system_dir)?;

    let mut state = WatchState::default();
    state.files.insert(
        "/tmp/a.md".to_string(),
        maestria_test_support::content_hash_str(10),
    );
    state.removed.insert(
        "/tmp/b.md".to_string(),
        maestria_test_support::content_hash_str(11),
    );
    state.artifact_ids.insert(
        "/tmp/a.md".to_string(),
        ArtifactIdEntry {
            artifact_id: 42,
            content_hash: maestria_test_support::content_hash_str(10),
        },
    );

    persist_state(&layout, &state)?;
    let loaded = load_state(&layout);
    assert_eq!(
        loaded.files.get("/tmp/a.md"),
        Some(maestria_test_support::content_hash_str(10)).as_ref()
    );
    assert_eq!(
        loaded.removed.get("/tmp/b.md"),
        Some(maestria_test_support::content_hash_str(11)).as_ref()
    );
    assert_eq!(
        loaded.artifact_ids.get("/tmp/a.md").map(|e| e.artifact_id),
        Some(42)
    );

    fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn corrupt_state_recovers_with_empty_state() -> Result<(), Box<dyn std::error::Error>> {
    let dir = env::temp_dir().join(format!("maestria-watcher-corrupt-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    let layout = InstanceLayout::for_root(dir.clone());
    fs::create_dir_all(&layout.system_dir)?;
    fs::write(
        layout.system_dir.join(WATCH_STATE_FILE),
        b"{ not valid watcher state",
    )?;

    let loaded = load_state(&layout);

    assert!(loaded.files.is_empty());
    assert!(loaded.removed.is_empty());
    assert!(loaded.artifact_ids.is_empty());
    fs::remove_dir_all(dir)?;
    Ok(())
}
