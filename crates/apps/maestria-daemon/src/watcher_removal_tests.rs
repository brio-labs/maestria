use super::*;

#[tokio::test]
async fn phase_detect_removals_emits_source_removed() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            files: [(
                "/tmp/new.md".to_string(),
                maestria_test_support::content_hash_str(1),
            )]
            .into_iter()
            .collect(),
            artifact_ids: [(
                "/tmp/old.md".to_string(),
                ArtifactIdEntry {
                    artifact_id: 42,
                    content_hash: maestria_test_support::content_hash_str(0),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    let previous_files = [(
        "/tmp/old.md".to_string(),
        maestria_test_support::content_hash_str(0),
    )]
    .into_iter()
    .collect();
    watcher.phase_detect_removals(previous_files).await?;
    assert!(
        watcher.state.removed.contains_key("/tmp/old.md"),
        "successfully delivered removal should remain as a durable tombstone"
    );
    let msg = input_rx
        .try_recv()
        .map_err(|_| "should emit SourceRemoved")?;
    assert!(
        matches!(&msg, DomainInput::SourceRemoved(input) if input.source_path == "/tmp/old.md")
    );
    Ok(())
}

#[tokio::test]
async fn phase_detect_removals_retries_after_channel_backpressure()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(1);
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: maestria_domain::ArtifactId::new(99),
            title: "filler".to_string(),
            source_path: "/tmp/filler".to_string(),
            source_bytes: Vec::new(),
            content_hash: maestria_test_support::content_hash(15)?,
        }))
        .await
        .map_err(|_| "fill the input channel")?;
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        shutdown: CancellationToken::new(),
        input_tx,
        artifact_ids: BTreeMap::new(),
        state: WatchState {
            files: [(
                "/tmp/old.md".to_string(),
                maestria_test_support::content_hash_str(0),
            )]
            .into_iter()
            .collect(),
            artifact_ids: [(
                "/tmp/old.md".to_string(),
                ArtifactIdEntry {
                    artifact_id: 42,
                    content_hash: maestria_test_support::content_hash_str(0),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };

    let previous_files = std::mem::take(&mut watcher.state.files);
    watcher.phase_detect_removals(previous_files).await?;
    assert!(
        watcher.state.removed.contains_key("/tmp/old.md"),
        "failed emission must retain the durable tombstone"
    );
    assert!(
        watcher.state.files.contains_key("/tmp/old.md"),
        "failed emission must remain in the next scan's previous files"
    );

    let filler = input_rx
        .try_recv()
        .map_err(|_| "expected the channel filler")?;
    assert!(matches!(filler, DomainInput::ArtifactDetected(_)));

    let previous_files = std::mem::take(&mut watcher.state.files);
    watcher.phase_detect_removals(previous_files).await?;
    let message = input_rx
        .try_recv()
        .map_err(|_| "expected retried SourceRemoved")?;
    assert!(
        matches!(
            &message,
            DomainInput::SourceRemoved(input) if input.source_path == "/tmp/old.md"
        ),
        "expected retried SourceRemoved, got {message:?}"
    );
    assert!(
        watcher.state.removed.contains_key("/tmp/old.md"),
        "successful retry must preserve the tombstone"
    );
    Ok(())
}

#[tokio::test]
async fn phase_detect_removals_detects_rename() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            files: [(
                "/tmp/renamed.md".to_string(),
                maestria_test_support::content_hash_str(10),
            )]
            .into_iter()
            .collect(),
            artifact_ids: [(
                "/tmp/old.md".to_string(),
                ArtifactIdEntry {
                    artifact_id: 42,
                    content_hash: maestria_test_support::content_hash_str(10),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    let previous_files = [(
        "/tmp/old.md".to_string(),
        maestria_test_support::content_hash_str(10),
    )]
    .into_iter()
    .collect();
    watcher.phase_detect_removals(previous_files).await?;
    assert!(
        watcher.state.removed.contains_key("/tmp/old.md"),
        "successfully delivered rename should remain as a durable tombstone"
    );
    let msg = input_rx
        .try_recv()
        .map_err(|_| "should emit SourceRemoved for old path")?;
    assert!(
        matches!(&msg, DomainInput::SourceRemoved(input) if input.source_path == "/tmp/old.md")
    );
    Ok(())
}

#[tokio::test]
async fn phase_detect_removals_cleans_up_stale_artifact_ids()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, _input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            files: [(
                "/tmp/current.md".to_string(),
                maestria_test_support::content_hash_str(10),
            )]
            .into_iter()
            .collect(),
            artifact_ids: [
                (
                    "/tmp/current.md".to_string(),
                    ArtifactIdEntry {
                        artifact_id: 1,
                        content_hash: maestria_test_support::content_hash_str(10),
                    },
                ),
                (
                    "/tmp/stale.md".to_string(),
                    ArtifactIdEntry {
                        artifact_id: 2,
                        content_hash: maestria_test_support::content_hash_str(2),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    watcher.phase_detect_removals(BTreeMap::new()).await?;
    assert!(watcher.state.artifact_ids.contains_key("/tmp/current.md"));
    assert!(
        !watcher.state.artifact_ids.contains_key("/tmp/stale.md"),
        "stale artifact_id should be cleaned up"
    );
    Ok(())
}

#[test]
fn emit_source_removed_returns_true_on_success() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            artifact_ids: [(
                "/tmp/old.md".to_string(),
                ArtifactIdEntry {
                    artifact_id: 42,
                    content_hash: maestria_test_support::content_hash_str(0),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    assert!(
        watcher
            .emit_source_removed("/tmp/old.md", &(maestria_test_support::content_hash_str(0)))?
    );
    let msg = input_rx
        .try_recv()
        .map_err(|_| "should receive SourceRemoved")?;
    assert!(
        matches!(&msg, DomainInput::SourceRemoved(input) if input.source_path == "/tmp/old.md")
    );
    Ok(())
}

#[test]
fn emit_source_removed_returns_false_when_channel_full() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, _input_rx) = mpsc::channel(1);
    input_tx
        .try_send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: maestria_domain::ArtifactId::new(99),
            title: "filler".to_string(),
            source_path: "/tmp/filler".to_string(),
            source_bytes: vec![],
            content_hash: maestria_test_support::content_hash(15)?,
        }))
        .map_err(|_| "fill the channel")?;
    let watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            artifact_ids: [(
                "/tmp/old.md".to_string(),
                ArtifactIdEntry {
                    artifact_id: 42,
                    content_hash: maestria_test_support::content_hash_str(0),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    assert!(
        !watcher
            .emit_source_removed("/tmp/old.md", &(maestria_test_support::content_hash_str(0)))?
    );
    Ok(())
}

#[test]
fn emit_source_removed_returns_false_when_artifact_id_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState::default(),
        pending: BTreeMap::new(),
        scan_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
    };
    assert!(!watcher.emit_source_removed("/tmp/unknown.md", "hash")?);
    assert!(input_rx.try_recv().is_err());
    Ok(())
}
