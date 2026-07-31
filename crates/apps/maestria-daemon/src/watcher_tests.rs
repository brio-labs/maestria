use super::*;
use std::{env, fs, path::PathBuf, process};
use tokio::sync::mpsc;

#[test]
fn scan_skips_instance_state_when_root_contains_instance() -> Result<(), Box<dyn std::error::Error>>
{
    let root = env::temp_dir().join(format!("maestria-watcher-instance-root-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("system"))?;
    fs::write(root.join("research.md"), "research")?;
    fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;

    let manifest = InstanceManifest {
        schema_version: 1,
        root: instance,
        read_roots: vec![root.clone()],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
    };
    let observations = scan_manifest(&manifest)?;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("research.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_preserves_relative_manifest_scope() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(format!(".maestria-watcher-relative-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("note.md"), "relative note")?;

    let manifest = test_manifest(root.clone());
    let observations = scan_manifest(&manifest)?;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_allows_read_root_nested_in_instance() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-nested-root-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    let nested = instance.join("workspace");
    fs::create_dir_all(&nested)?;
    fs::write(nested.join("note.md"), "nested note")?;

    let manifest = InstanceManifest {
        schema_version: 1,
        root: instance,
        read_roots: vec![nested],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
    };
    let observations = scan_manifest(&manifest)?;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_excludes_instance_manifest_and_preserves_alias_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-instance-alias-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("system"))?;
    fs::create_dir_all(instance.join("workspace"))?;
    fs::write(instance.join("manifest.txt"), "root=/tmp/secret")?;
    fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;
    fs::write(instance.join("workspace").join("note.md"), "user note")?;

    let manifest = InstanceManifest {
        schema_version: 1,
        root: instance.clone(),
        read_roots: vec![instance.join(".")],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
    };
    let observations = scan_manifest(&manifest)?;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("workspace/note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_is_deterministic_and_skips_sensitive_files() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-test-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("note.md"), "note")?;
    fs::write(root.join(".env"), "secret")?;
    let manifest = test_manifest(root.clone());
    let first = scan_manifest(&manifest)?;
    let second = scan_manifest(&manifest)?;
    assert_eq!(
        first.iter().map(|item| &item.path).collect::<Vec<_>>(),
        second.iter().map(|item| &item.path).collect::<Vec<_>>()
    );
    assert_eq!(first.len(), 1);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_respects_gitignore() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-gitignore-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("tracked.md"), "tracked content")?;
    fs::write(root.join("ignored.md"), "ignored content")?;
    fs::write(root.join(".gitignore"), "ignored.md")?;
    let manifest = test_manifest(root.clone());
    let observations = scan_manifest(&manifest)?;
    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].path.ends_with("tracked.md"),
        "only tracked.md should appear, got: {:?}",
        observations[0].path
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_respects_ignore_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-ignore-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("ok.md"), "ok")?;
    fs::write(root.join("ignored.md"), "should be ignored")?;
    fs::write(root.join(".ignore"), "ignored.md")?;
    let manifest = test_manifest(root.clone());
    let observations = scan_manifest(&manifest)?;
    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].path.ends_with("ok.md"),
        "only ok.md should appear, got: {:?}",
        observations[0].path
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn state_persistence_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let dir = env::temp_dir().join(format!("maestria-watcher-state-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;

    let layout = InstanceLayout::for_root(dir.clone());
    fs::create_dir_all(&layout.system_dir)?;

    let mut state = WatchState::default();
    state
        .files
        .insert("/tmp/a.md".to_string(), "hash1".to_string());
    state
        .removed
        .insert("/tmp/b.md".to_string(), "hash2".to_string());
    state.artifact_ids.insert(
        "/tmp/a.md".to_string(),
        ArtifactIdEntry {
            artifact_id: 42,
            content_hash: "hash1".to_string(),
        },
    );

    persist_state(&layout, &state)?;
    let loaded = load_state(&layout);
    assert_eq!(loaded.files.get("/tmp/a.md"), Some(&"hash1".to_string()));
    assert_eq!(loaded.removed.get("/tmp/b.md"), Some(&"hash2".to_string()));
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
        (accepted.artifact_id, accepted.content_hash.clone()),
    );
    watcher
        .state
        .files
        .insert(accepted.source_path.clone(), accepted.content_hash.clone());
    watcher.state.artifact_ids.insert(
        accepted.source_path.clone(),
        ArtifactIdEntry {
            artifact_id: accepted.artifact_id.value(),
            content_hash: accepted.content_hash.clone(),
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
        (accepted.artifact_id, accepted.content_hash.clone()),
    );
    watcher
        .state
        .files
        .insert(accepted.source_path.clone(), accepted.content_hash.clone());
    watcher.state.artifact_ids.insert(
        accepted.source_path.clone(),
        ArtifactIdEntry {
            artifact_id: accepted.artifact_id.value(),
            content_hash: accepted.content_hash.clone(),
        },
    );
    watcher.pending.remove(&accepted.source_path);
    persist_state(&layout, &watcher.state)?;

    // Simulate restart after runtime acceptance.
    let loaded_state = load_state(&layout);
    assert_eq!(
        loaded_state.files.get(&accepted.source_path),
        Some(&accepted.content_hash),
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
        (accepted.artifact_id, accepted.content_hash.clone()),
    );
    watcher
        .state
        .files
        .insert(accepted.source_path.clone(), accepted.content_hash.clone());
    watcher.state.artifact_ids.insert(
        accepted.source_path.clone(),
        ArtifactIdEntry {
            artifact_id: accepted.artifact_id.value(),
            content_hash: accepted.content_hash.clone(),
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

#[tokio::test]
async fn phase_detect_additions_emits_for_new_file() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp")),
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };
    let obs = Observation {
        path: PathBuf::from("/tmp/new.md"),
        bytes: b"content".to_vec(),
        hash: "hash1".to_string(),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert!(current.contains_key("/tmp/new.md"));
    let msg = input_rx
        .try_recv()
        .map_err(|_| "should have emitted ArtifactDetected")?;
    assert!(
        matches!(&msg, DomainInput::ArtifactDetected(input) if input.source_path == "/tmp/new.md")
    );
    assert_eq!(
        watcher.pending.get("/tmp/new.md"),
        Some(&PendingDelivery {
            content_hash: "hash1".to_string(),
            status: PendingDeliveryStatus::Enqueued,
        })
    );
    let _ = watcher
        .phase_detect_additions(&[Observation {
            path: PathBuf::from("/tmp/new.md"),
            bytes: b"content".to_vec(),
            hash: "hash1".to_string(),
        }])
        .await?;
    assert!(
        input_rx.try_recv().is_err(),
        "same-session pending delivery must not be duplicated"
    );
    Ok(())
}
#[tokio::test]
async fn phase_detect_additions_skips_unchanged_file() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp")),
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            files: [("/tmp/existing.md".to_string(), "hash1".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };
    let obs = Observation {
        path: PathBuf::from("/tmp/existing.md"),
        bytes: b"content".to_vec(),
        hash: "hash1".to_string(),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert_eq!(current.get("/tmp/existing.md"), Some(&"hash1".to_string()));
    assert!(
        input_rx.try_recv().is_err(),
        "should not emit for unchanged file"
    );
    Ok(())
}

#[tokio::test]
async fn phase_detect_additions_skips_matching_artifact_id_and_hash()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp")),
        input_tx,
        artifact_ids: [(
            "/tmp/existing.md".to_string(),
            (maestria_domain::ArtifactId::new(1), "hash1".to_string()),
        )]
        .into_iter()
        .collect(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };
    let obs = Observation {
        path: PathBuf::from("/tmp/existing.md"),
        bytes: b"content".to_vec(),
        hash: "hash1".to_string(),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert_eq!(current.get("/tmp/existing.md"), Some(&"hash1".to_string()));
    assert!(
        input_rx.try_recv().is_err(),
        "should not emit when artifact_id and hash match"
    );
    Ok(())
}

#[tokio::test]
async fn phase_detect_additions_respects_backpressure() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(1);
    // Fill the single buffer slot so capacity() becomes 0.
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: maestria_domain::ArtifactId::new(99),
            title: "filler".to_string(),
            source_path: "/tmp/filler".to_string(),
            source_bytes: vec![],
            content_hash: "filler".to_string(),
        }))
        .await?;
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp")),
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };
    let obs = Observation {
        path: PathBuf::from("/tmp/backpressure.md"),
        bytes: b"content".to_vec(),
        hash: "hash_bp".to_string(),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert!(
        current.contains_key("/tmp/backpressure.md"),
        "physical source presence must remain visible while channel is full"
    );
    assert_eq!(
        watcher.pending.get("/tmp/backpressure.md"),
        Some(&PendingDelivery {
            content_hash: "hash_bp".to_string(),
            status: PendingDeliveryStatus::Deferred,
        })
    );
    // Only the filler message should be present.
    assert!(input_rx.try_recv().is_ok());
    assert!(input_rx.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn phase_detect_additions_reports_closed_input_channel()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, input_rx) = mpsc::channel(1);
    drop(input_rx);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp")),
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };
    let obs = Observation {
        path: PathBuf::from("/tmp/closed.md"),
        bytes: b"content".to_vec(),
        hash: "hash_closed".to_string(),
    };

    let result = watcher.phase_detect_additions(&[obs]).await;

    assert!(result.is_err(), "closed input channel must be reported");
    Ok(())
}

#[tokio::test]
async fn phase_detect_additions_full_channel_completes_without_false_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, _input_rx) = mpsc::channel(1);
    // Fill the single buffer slot so the channel is full.
    input_tx
        .try_send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: maestria_domain::ArtifactId::new(99),
            title: "filler".to_string(),
            source_path: "/tmp/filler".to_string(),
            source_bytes: vec![],
            content_hash: "filler".to_string(),
        }))
        .map_err(|_| "fill the channel")?;
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp")),
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        pending: BTreeMap::new(),
    };
    let obs = Observation {
        path: PathBuf::from("/tmp/race.md"),
        bytes: b"content".to_vec(),
        hash: "hash_race".to_string(),
    };

    // Must complete without hanging even though the channel is full.
    let current = tokio::time::timeout(
        Duration::from_secs(1),
        watcher.phase_detect_additions(&[obs]),
    )
    .await
    .map_err(|_| "phase_detect_additions hung on full channel")??;

    assert!(
        current.contains_key("/tmp/race.md"),
        "deferred source remains physically present"
    );
    assert!(
        !watcher.artifact_ids.contains_key("/tmp/race.md"),
        "deferred file must not get an artifact_id committed"
    );
    assert!(
        !watcher.state.artifact_ids.contains_key("/tmp/race.md"),
        "deferred file must not get a state artifact_id committed"
    );

    Ok(())
}
