use super::*;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Evidence, EvidenceKind, IndexStatus, LogicalTick,
    ParseArtifactRequest, StoreBlobRequest,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier};
use maestria_ports::{
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex,
    InMemoryHarnessAdapter, InMemoryParser, InMemoryVectorIndex, InMemoryWebFetcher, PortError,
};
use std::collections::BTreeSet;
use std::sync::Arc;

/// BlobStore that records every `put` payload for later assertion.
struct RecordingBlobStore {
    recorded: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
}

impl BlobStore for RecordingBlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
        match self.recorded.lock() {
            Ok(mut guard) => {
                let id = guard.len() as u64 + 1;
                guard.push(bytes);
                Ok(BlobId::new(id))
            }
            Err(_poisoned) => Err(PortError::Internal {
                message: "recording blob store lock poisoned".to_string(),
            }),
        }
    }

    fn get(&self, _id: BlobId) -> Result<Vec<u8>, PortError> {
        Err(PortError::NotFound)
    }
}

#[tokio::test]
async fn store_blob_passes_exact_payload_bytes() {
    let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let blob_store = Arc::new(RecordingBlobStore {
        recorded: recorded.clone(),
    });
    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store,
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let expected = b"exact payload content".to_vec();
    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::StoreBlob(StoreBlobRequest {
            artifact_id: ArtifactId::new(1),
            payload: expected.clone(),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(result, "StoreBlob should succeed");
    match recorded.lock() {
        Ok(guard) => {
            assert_eq!(
                guard.len(),
                0,
                "StoreBlob no longer calls put; blob stored during ParseArtifact"
            );
        }
        Err(poisoned) => panic!("recording mutex poisoned: {:?}", poisoned),
    }
}

#[tokio::test]
async fn parse_artifact_calls_blob_put_exactly_once() {
    let recorded: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let blob_store = Arc::new(RecordingBlobStore {
        recorded: recorded.clone(),
    });

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(55),
            title: "single-put".into(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store,
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(artifact_repo),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let source_bytes = b"single put test content".to_vec();
    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(55),
            source_path: "/repo/single.rs".to_string(),
            source_bytes: source_bytes.clone(),
            source_blob: None,
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    )
    .await;

    assert!(result, "ParseArtifact should succeed");
    match recorded.lock() {
        Ok(guard) => {
            assert_eq!(guard.len(), 1, "exactly one blob put per ParseArtifact");
            assert_eq!(guard[0], source_bytes, "payload matches");
        }
        Err(poisoned) => panic!("recording mutex poisoned: {:?}", poisoned),
    }
}

#[tokio::test]
async fn parse_artifact_retry_skips_existing_evidence() {
    let artifact_id = ArtifactId::new(77);
    let existing_evidence_id = evidence_id_for(artifact_id, 0);

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: artifact_id,
            title: "retry-test".into(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::from([existing_evidence_id]),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    // Pre-populate state with existing evidence to simulate replay/retry.
    let mut state = KernelState::new();
    state.evidences.insert(
        existing_evidence_id,
        Evidence {
            id: existing_evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/repo/retry.rs".into(),
                range: maestria_domain::ContentRange { start: 1, end: 1 },
                content_hash: content_hash(b"existing"),
                snapshot: Some(BlobId::new(1)),
            },
            excerpt: "existing excerpt".into(),
            observed_at: LogicalTick::new(1),
        },
    );

    let adapters = Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(artifact_repo),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id,
            source_path: "/repo/retry.rs".to_string(),
            source_bytes: b"retry content".to_vec(),
            source_blob: None,
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(state)),
        input_tx,
    )
    .await;

    assert!(result, "ParseArtifact should succeed even on retry");

    // First input after retry: ParserStarted (fresh ingestion sends it before parsing).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(_ps))) => {
            // ParserStarted acknowledged.
        }
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            panic!("unexpected RecordEvidence before ParserStarted for evidence_id {ev:?}");
        }
        Ok(Some(other)) => panic!("expected ParserStarted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserStarted"),
        Err(_) => panic!("timeout waiting for ParserStarted"),
    }

    // Second input: ParserCompleted (no RecordEvidence for already-present evidence).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, artifact_id);
            assert_eq!(pr.chunks.len(), 1);
        }
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            panic!("unexpected RecordEvidence for already-present evidence_id {ev:?}");
        }
        Ok(Some(other)) => panic!("expected ParserCompleted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserCompleted"),
        Err(_) => panic!("timeout waiting for ParserCompleted"),
    }
}
