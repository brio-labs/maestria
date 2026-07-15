use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, DomainEvent, DomainEventEnvelope, EventId, IndexStatus,
    ParseArtifactRequest, SequenceNumber, content_hash,
};
use maestria_ports::{
    BlobStore, EventLog, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryEventLog,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

#[tokio::test]
async fn parse_artifact_barrier_blocks_parse_until_persistence_observable() {
    let event_log = Arc::new(InMemoryEventLog::new());
    let artifact_id = ArtifactId::new(99);
    let source_bytes = b"fn main() {}".to_vec();
    let source_hash = content_hash(&source_bytes);

    // Store the blob and record its blob_id so the pre-populated event
    // carries the same identity the handler will compute.
    let blob_store = InMemoryBlobStore::new();
    let blob_id = blob_store
        .put(source_bytes.clone())
        .expect("put should succeed");

    // Populate the event log with a ParserStarted envelope carrying the
    // exact artifact_id, blob_id, _and_ content_hash that the handler
    // will later send. A stale envelope from a prior attempt with different
    // content must never satisfy the barrier.
    let _ = event_log.append(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id,
            title: "barrier-test".to_string(),
            source_path: "/repo/barrier.rs".to_string(),
            content_hash: source_hash.clone(),
            blob_id,
        },
    });

    // Use the same blob_store so the handler's put is idempotent
    // (InMemoryBlobStore returns the same BlobId for equal content).
    let adapters = Adapters {
        event_log: event_log.clone(),
        blob_store: Arc::new(blob_store),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    // With a populated event log, the barrier should find the event and
    // parsing should succeed even with a tight timeout (production path).
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/barrier.rs".to_string(),
            source_bytes,
            source_blob: None,
        }),
        ctx,
        Some(Duration::from_millis(500)),
    )
    .await;

    assert!(
        result,
        "ParseArtifact should succeed when ParserStarted is already observable"
    );
}

#[tokio::test]
async fn parse_artifact_barrier_timeout_without_persistence_returns_failure() {
    // Empty event log — nobody reads the input channel, so the sent
    // ParserStarted input is never persisted. The barrier must time out
    // and parsing must be skipped.
    let event_log = Arc::new(InMemoryEventLog::new());
    let artifact_id = ArtifactId::new(42);

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: artifact_id,
            title: "timeout-test".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
            parse_status: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        event_log: event_log.clone(),
        artifact_repo: Arc::new(artifact_repo),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/timeout.rs".to_string(),
            source_bytes: b"fn main() {}".to_vec(),
            source_blob: None,
        }),
        ctx,
        Some(Duration::from_millis(100)),
    )
    .await;

    assert!(
        !result,
        "ParseArtifact with persistence barrier must fail when ParserStarted is never persisted"
    );
}
