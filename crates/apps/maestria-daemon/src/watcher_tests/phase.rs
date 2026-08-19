use super::*;
use maestria_domain::ArtifactDetected;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[tokio::test]
async fn phase_detect_additions_emits_for_new_file() -> Result<(), Box<dyn std::error::Error>> {
    let (input_tx, mut input_rx) = mpsc::channel(256);
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
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
        hash: maestria_test_support::content_hash_str(10),
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
            content_hash: maestria_test_support::content_hash_str(10),
            status: PendingDeliveryStatus::Enqueued,
        })
    );
    watcher
        .phase_detect_additions(&[Observation {
            path: PathBuf::from("/tmp/new.md"),
            bytes: b"content".to_vec(),
            hash: maestria_test_support::content_hash_str(10),
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
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: BTreeMap::new(),
        shutdown: CancellationToken::new(),
        state: WatchState {
            files: [(
                "/tmp/existing.md".to_string(),
                maestria_test_support::content_hash_str(10),
            )]
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
        hash: maestria_test_support::content_hash_str(10),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert_eq!(
        current.get("/tmp/existing.md"),
        Some(maestria_test_support::content_hash_str(10)).as_ref()
    );
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
        manifest: test_manifest(PathBuf::from("/tmp"))?,
        input_tx,
        artifact_ids: [(
            "/tmp/existing.md".to_string(),
            (
                maestria_domain::ArtifactId::new(1),
                maestria_test_support::content_hash_str(10),
            ),
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
        hash: maestria_test_support::content_hash_str(10),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert_eq!(
        current.get("/tmp/existing.md"),
        Some(maestria_test_support::content_hash_str(10)).as_ref()
    );
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
            content_hash: maestria_test_support::content_hash(15)?,
        }))
        .await?;
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
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
        hash: maestria_test_support::content_hash_str(3),
    };
    let current = watcher.phase_detect_additions(&[obs]).await?;
    assert!(
        current.contains_key("/tmp/backpressure.md"),
        "physical source presence must remain visible while channel is full"
    );
    assert_eq!(
        watcher.pending.get("/tmp/backpressure.md"),
        Some(&PendingDelivery {
            content_hash: maestria_test_support::content_hash_str(3),
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
        manifest: test_manifest(PathBuf::from("/tmp"))?,
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
        hash: maestria_test_support::content_hash_str(4),
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
            content_hash: maestria_test_support::content_hash(15)?,
        }))
        .map_err(|_| "fill the channel")?;
    let mut watcher = Watcher {
        layout: InstanceLayout::for_root(PathBuf::from("/tmp")),
        manifest: test_manifest(PathBuf::from("/tmp"))?,
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
        hash: maestria_test_support::content_hash_str(5),
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
