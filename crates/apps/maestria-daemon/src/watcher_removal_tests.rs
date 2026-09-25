use super::*;
use maestria_domain::ArtifactDetected;
use maestria_ports::EventLog;

#[tokio::test]
async fn phase_detect_removals_emits_source_removed() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: Arc::new(RwLock::new(test_manifest(PathBuf::from("/tmp"))?)),
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
        receipts: test_receipts()?,
    };
    let previous_files = [(
        "/tmp/old.md".to_string(),
        maestria_test_support::content_hash_str(0),
    )]
    .into_iter()
    .collect();
    let previous_ids = watcher.state.artifact_ids.clone();
    watcher
        .phase_detect_removals(previous_files, previous_ids)
        .await?;
    watcher.phase_process_pending_removals()?;
    let removal_key = pending_removal_key("/tmp/old.md", 42);
    assert!(watcher.state.pending_removals.contains_key(&removal_key));
    assert!(
        input_rx.try_recv().is_err(),
        "an unaccepted artifact must not be revoked"
    );
    append_parser_started(
        &watcher,
        42,
        "/tmp/old.md",
        &maestria_test_support::content_hash_str(0),
    )?;
    watcher.phase_process_pending_removals()?;
    assert!(!watcher.state.removed.contains_key("/tmp/old.md"));
    let msg = input_rx
        .try_recv()
        .map_err(|_| "should emit SourceRemoved")?;
    assert!(matches!(&msg, DomainInput::SourceRemoved(input)
            if input.source_path == "/tmp/old.md"
                && input.artifact_id == maestria_domain::ArtifactId::new(42)));
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
        manifest: Arc::new(RwLock::new(test_manifest(PathBuf::from("/tmp"))?)),
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
        receipts: test_receipts()?,
    };
    append_parser_started(
        &watcher,
        42,
        "/tmp/old.md",
        &maestria_test_support::content_hash_str(0),
    )?;

    let previous_ids = watcher.state.artifact_ids.clone();
    let previous_files = std::mem::take(&mut watcher.state.files);
    watcher
        .phase_detect_removals(previous_files, previous_ids)
        .await?;
    watcher.phase_process_pending_removals()?;
    let removal_key = pending_removal_key("/tmp/old.md", 42);
    assert!(watcher.state.pending_removals.contains_key(&removal_key));
    assert!(!watcher.state.removed.contains_key("/tmp/old.md"));
    assert!(!watcher.state.files.contains_key("/tmp/old.md"));

    let filler = input_rx
        .try_recv()
        .map_err(|_| "expected the channel filler")?;
    assert!(matches!(filler, DomainInput::ArtifactDetected(_)));

    watcher.phase_process_pending_removals()?;
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
    assert!(watcher.state.pending_removals.contains_key(&removal_key));
    Ok(())
}

#[tokio::test]
async fn phase_detect_removals_detects_rename() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: Arc::new(RwLock::new(test_manifest(PathBuf::from("/tmp"))?)),
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
        receipts: test_receipts()?,
    };
    append_parser_started(
        &watcher,
        42,
        "/tmp/old.md",
        &maestria_test_support::content_hash_str(10),
    )?;
    let previous_files = [(
        "/tmp/old.md".to_string(),
        maestria_test_support::content_hash_str(10),
    )]
    .into_iter()
    .collect();
    let previous_ids = watcher.state.artifact_ids.clone();
    watcher
        .phase_detect_removals(previous_files, previous_ids)
        .await?;
    watcher.phase_process_pending_removals()?;
    assert!(
        watcher
            .state
            .pending_removals
            .contains_key(&pending_removal_key("/tmp/old.md", 42))
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
async fn phase_detect_removals_queues_stale_artifact_ids() -> Result<(), Box<dyn std::error::Error>>
{
    let (input_tx, _input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: Arc::new(RwLock::new(test_manifest(PathBuf::from("/tmp"))?)),
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
        receipts: test_receipts()?,
    };
    let previous_ids = watcher.state.artifact_ids.clone();
    watcher
        .phase_detect_removals(BTreeMap::new(), previous_ids)
        .await?;
    assert!(watcher.state.artifact_ids.contains_key("/tmp/current.md"));
    assert!(
        watcher
            .state
            .pending_removals
            .contains_key(&pending_removal_key("/tmp/stale.md", 2))
    );
    Ok(())
}

fn append_parser_started(
    watcher: &Watcher,
    artifact_id: u64,
    source_path: &str,
    content_hash: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    EventLog::append(
        &watcher.receipts.event_log,
        maestria_domain::DomainEventEnvelope {
            id: maestria_domain::EventId::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id: maestria_domain::ArtifactId::new(artifact_id),
                title: "test artifact".to_owned(),
                source_path: source_path.to_owned(),
                content_hash: maestria_domain::ContentHash::new(content_hash.to_owned())?,
                blob_id: maestria_domain::BlobId::new(1),
            },
        },
    )?;
    Ok(())
}
