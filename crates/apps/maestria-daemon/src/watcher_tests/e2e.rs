use super::*;
use std::{env, fs, process};
use tokio::sync::mpsc;

#[tokio::test]
async fn scan_once_detects_creation_and_removal() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-e2e-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let shutdown = CancellationToken::new();

    fs::write(root.join("hello.md"), "hello world")?;

    let manifest = test_manifest(root.clone());
    let state = WatchState::default();
    let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(root.clone()),
        manifest,
        input_tx: input_tx.clone(),
        artifact_ids: BTreeMap::new(),
        shutdown: shutdown.clone(),
        state,
        scan_permits,
        pending: BTreeMap::new(),
    };
    watcher.scan_once().await?;
    let detected = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await?
        .ok_or("watcher input channel closed")?;
    assert!(
        matches!(&detected, DomainInput::ArtifactDetected(input) if input.source_path.ends_with("hello.md")),
        "expected ArtifactDetected for hello.md, got {detected:?}"
    );
    // A bounded-channel enqueue is not acceptance. Simulate the runtime
    // persisting its ParserStarted identity before the source is removed.
    let accepted = match &detected {
        DomainInput::ArtifactDetected(input) => input,
        other => return Err(format!("expected ArtifactDetected, got {other:?}").into()),
    };
    watcher.artifact_ids.insert(
        accepted.source_path.clone(),
        (
            accepted.artifact_id,
            accepted.content_hash.as_str().to_owned(),
        ),
    );
    watcher.state.files.insert(
        accepted.source_path.clone(),
        accepted.content_hash.as_str().to_owned(),
    );
    watcher.state.artifact_ids.insert(
        accepted.source_path.clone(),
        ArtifactIdEntry {
            artifact_id: accepted.artifact_id.value(),
            content_hash: accepted.content_hash.as_str().to_owned(),
        },
    );
    watcher.pending.remove(&accepted.source_path);

    // Remove the file and add a different one.
    fs::remove_file(root.join("hello.md"))?;
    fs::write(root.join("other.md"), "other content")?;

    // Scan again.
    watcher.scan_once().await?;

    let mut found_removed = false;
    let mut found_other = false;
    for _ in 0..4 {
        match tokio::time::timeout(Duration::from_secs(5), input_rx.recv()).await {
            Ok(Some(DomainInput::SourceRemoved(input))) => {
                found_removed = true;
                assert!(
                    input.source_path.ends_with("hello.md"),
                    "expected SourceRemoved for hello.md, got {input:?}"
                );
            }
            Ok(Some(DomainInput::ArtifactDetected(input)))
                if input.source_path.ends_with("other.md") =>
            {
                found_other = true;
            }
            Ok(None) => break,
            Err(_) => break,
            _ => {}
        }
    }

    assert!(
        found_removed,
        "SourceRemoved was not emitted for removed file"
    );
    assert!(found_other, "ArtifactDetected was not emitted for new file");

    shutdown.cancel();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn changed_file_gets_new_artifact_identity_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-change-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let path = root.join("changed.md");
    fs::write(&path, "initial content")?;

    let (input_tx, mut input_rx) = mpsc::channel(256);
    let layout = InstanceLayout::for_root(root.clone());
    let manifest = test_manifest(root.clone());
    let mut first_watcher = Watcher {
        layout: layout.clone(),
        manifest: manifest.clone(),
        input_tx: input_tx.clone(),
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };

    first_watcher.scan_once().await?;
    let first = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await?
        .ok_or("watcher input channel closed")?;
    let first_id = match first {
        DomainInput::ArtifactDetected(input) => input.artifact_id,
        other => return Err(format!("expected first artifact detection, got {other:?}").into()),
    };
    let artifact_ids = first_watcher.artifact_ids.clone();

    fs::write(&path, "updated content")?;
    let mut restarted_watcher = Watcher {
        layout: layout.clone(),
        manifest,
        input_tx,
        artifact_ids,
        shutdown: CancellationToken::new(),
        state: load_state(&layout),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };

    restarted_watcher.scan_once().await?;
    let changed = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await?
        .ok_or("watcher input channel closed after restart")?;
    let changed_id = match changed {
        DomainInput::ArtifactDetected(input) => input.artifact_id,
        other => {
            return Err(format!(
                "expected changed artifact detection after restart, got {other:?}"
            )
            .into());
        }
    };

    assert_ne!(
        first_id, changed_id,
        "changed content must create a new artifact version identity after restart"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn state_persistence_survives_restart() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-restart-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let layout = InstanceLayout::for_root(root.clone());
    fs::create_dir_all(&layout.system_dir)?;

    // Simulate first daemon session.
    fs::write(root.join("survive.md"), "hello")?;
    let (tx, mut rx) = mpsc::channel(256);
    let shutdown = CancellationToken::new();
    let manifest = test_manifest(root.clone());
    let state = WatchState::default();
    let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
    let mut watcher = Watcher {
        layout: layout.clone(),
        manifest: manifest.clone(),
        input_tx: tx.clone(),
        artifact_ids: BTreeMap::new(),
        shutdown: shutdown.clone(),
        state,
        scan_permits: scan_permits.clone(),
        pending: BTreeMap::new(),
    };

    // Enqueueing alone leaves the source outside durable state.
    watcher.scan_once().await?;
    let state_path = layout.system_dir.join(WATCH_STATE_FILE);
    assert!(state_path.exists(), "watch state must persist after scan");
    assert!(
        load_state(&layout).files.is_empty(),
        "channel enqueue must not persist acceptance"
    );

    // Simulate the runtime's durable acceptance boundary using its replayed
    // artifact identity, then persist the accepted watcher state.
    let accepted = match rx.try_recv().map_err(|_| "expected ArtifactDetected")? {
        DomainInput::ArtifactDetected(input) => input,
        other => return Err(format!("expected ArtifactDetected, got {other:?}").into()),
    };
    watcher.artifact_ids.insert(
        accepted.source_path.clone(),
        (
            accepted.artifact_id,
            accepted.content_hash.as_str().to_owned(),
        ),
    );
    watcher.state.files.insert(
        accepted.source_path.clone(),
        accepted.content_hash.as_str().to_owned(),
    );
    watcher.state.artifact_ids.insert(
        accepted.source_path.clone(),
        ArtifactIdEntry {
            artifact_id: accepted.artifact_id.value(),
            content_hash: accepted.content_hash.as_str().to_owned(),
        },
    );
    watcher.pending.remove(&accepted.source_path);
    persist_state(&layout, &watcher.state)?;

    // Simulate restart after runtime acceptance.
    let loaded_state = load_state(&layout);
    assert_eq!(
        loaded_state
            .files
            .get(&accepted.source_path)
            .map(|hash| hash.as_str()),
        Some(accepted.content_hash.as_str()),
        "runtime-accepted source must remain tracked after restart"
    );
    assert_eq!(
        loaded_state
            .artifact_ids
            .get(&accepted.source_path)
            .map(|entry| entry.artifact_id),
        Some(accepted.artifact_id.value()),
        "runtime-accepted artifact identity must remain durable"
    );

    shutdown.cancel();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn enqueued_delivery_is_retried_after_cancelled_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-retry-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("retry.md"), "retry me")?;
    let layout = InstanceLayout::for_root(root.clone());
    fs::create_dir_all(&layout.system_dir)?;
    let manifest = test_manifest(root.clone());

    let (input_tx, mut input_rx) = mpsc::channel(1);
    let shutdown = CancellationToken::new();
    let mut watcher = Watcher {
        layout: layout.clone(),
        manifest: manifest.clone(),
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: shutdown.clone(),
        state: WatchState::default(),
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };

    watcher.scan_once().await?;
    let first = input_rx
        .try_recv()
        .map_err(|_| "expected the buffered artifact delivery")?;
    let (first_path, first_hash) = match &first {
        DomainInput::ArtifactDetected(input) => {
            (input.source_path.clone(), input.content_hash.clone())
        }
        other => return Err(format!("expected ArtifactDetected, got {other:?}").into()),
    };
    assert!(
        load_state(&layout).files.is_empty(),
        "channel enqueue must not persist acceptance"
    );

    // Model cancellation after enqueue but before runtime-side acceptance.
    shutdown.cancel();
    drop(input_rx);
    drop(watcher);

    let (retry_tx, mut retry_rx) = mpsc::channel(1);
    let mut restarted = Watcher {
        layout: layout.clone(),
        manifest,
        input_tx: retry_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: load_state(&layout),
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    restarted.scan_once().await?;
    let retry = retry_rx
        .try_recv()
        .map_err(|_| "expected the source to be retried after restart")?;
    assert!(
        matches!(
            &retry,
            DomainInput::ArtifactDetected(input)
                if input.source_path == first_path && input.content_hash == first_hash
        ),
        "cancelled delivery must be emitted again after restart: {retry:?}"
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn rename_emits_source_removed_for_old_path() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-rename-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let shutdown = CancellationToken::new();

    // Seed a file and scan once.
    fs::write(root.join("original.md"), "same content")?;
    let manifest = test_manifest(root.clone());
    let state = WatchState::default();
    let scan_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS));
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(root.clone()),
        manifest: manifest.clone(),
        input_tx: input_tx.clone(),
        artifact_ids: BTreeMap::new(),
        shutdown: shutdown.clone(),
        state,
        pending: BTreeMap::new(),
        scan_permits,
    };

    watcher.scan_once().await?;
    let detected = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await?
        .ok_or("watcher input channel closed")?;
    let accepted = match detected {
        DomainInput::ArtifactDetected(input) => input,
        other => return Err(format!("expected ArtifactDetected, got {other:?}").into()),
    };
    watcher.artifact_ids.insert(
        accepted.source_path.clone(),
        (
            accepted.artifact_id,
            accepted.content_hash.as_str().to_owned(),
        ),
    );
    watcher.state.files.insert(
        accepted.source_path.clone(),
        accepted.content_hash.as_str().to_owned(),
    );
    watcher.state.artifact_ids.insert(
        accepted.source_path.clone(),
        ArtifactIdEntry {
            artifact_id: accepted.artifact_id.value(),
            content_hash: accepted.content_hash.as_str().to_owned(),
        },
    );
    watcher.pending.remove(&accepted.source_path);
    persist_state(&watcher.layout, &watcher.state)?;

    // "Rename" by creating a new file with same content and removing old one.
    fs::write(root.join("renamed.md"), "same content")?;
    fs::remove_file(root.join("original.md"))?;

    // Reload persisted state to simulate fresh scan.
    watcher.state = load_state(&watcher.layout);
    watcher.scan_once().await?;

    // Should see SourceRemoved for original.md.
    let mut found = false;
    for _ in 0..4 {
        match tokio::time::timeout(Duration::from_secs(5), input_rx.recv()).await {
            Ok(Some(DomainInput::SourceRemoved(input))) => {
                if input.source_path.contains("original.md") {
                    found = true;
                }
            }
            Ok(Some(DomainInput::ArtifactDetected(_))) => {}
            _ => break,
        }
    }

    assert!(found, "rename should emit SourceRemoved for original path");

    shutdown.cancel();
    fs::remove_dir_all(root)?;
    Ok(())
}
