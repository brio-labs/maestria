use super::*;
use maestria_domain::{Artifact, ArtifactId, BlobId, IndexStatus, ParseArtifactRequest};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier};
use maestria_ports::{
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex,
    InMemoryHarnessAdapter, InMemoryParser, InMemoryVectorIndex, InMemoryWebFetcher,
};
use std::collections::BTreeSet;
use std::sync::Arc;

#[tokio::test]
async fn resume_parse_uses_existing_blob_and_skips_storage() {
    // Pre-populate the blob store with known bytes so the resume path can
    // fetch them by blob ID instead of storing fresh bytes.
    let blob_store = Arc::new(InMemoryBlobStore::new());
    let resume_bytes = b"resume parse content".to_vec();
    let blob_id = blob_store
        .put(resume_bytes.clone())
        .expect("pre-populate blob");

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(200),
            title: "resume-artifact".into(),
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
        blob_store: blob_store.clone(),
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
            artifact_id: ArtifactId::new(200),
            source_path: "/repo/resume.rs".to_string(),
            source_bytes: Vec::new(), // empty — bytes come from blob store
            source_blob: Some(blob_id),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
        None,
    )
    .await;

    assert!(result, "resume ParseArtifact should succeed");

    // Resume must NOT send ParserStarted (already persisted on first attempt).
    // First input should be ParserCompleted.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, ArtifactId::new(200));
            assert_eq!(pr.chunks.len(), 1);
            assert_eq!(pr.chunks[0].text, "resume parse content");
        }
        Ok(Some(DomainInput::ParserStarted(_))) => {
            panic!("resume must not send ParserStarted again");
        }
        Ok(Some(other)) => panic!("expected ParserCompleted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserCompleted"),
        Err(_) => panic!("timeout waiting for ParserCompleted"),
    }
}

#[tokio::test]
async fn resume_parse_missing_blob_returns_failure() {
    let blob_id = BlobId::new(999);

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(201),
            title: "missing-blob-artifact".into(),
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
    let (input_tx, _input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(201),
            source_path: "/repo/missing.rs".to_string(),
            source_bytes: Vec::new(),
            source_blob: Some(blob_id),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
        None,
    )
    .await;

    assert!(
        !result,
        "resume with missing blob must return false for retry"
    );
}

#[tokio::test]
async fn fresh_parse_sends_parser_started_with_correct_blob_identity() {
    let blob_store = Arc::new(InMemoryBlobStore::new());

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: ArtifactId::new(202),
            title: "fresh-artifact".into(),
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
        blob_store: blob_store.clone(),
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

    let source_bytes = b"fresh blob identity test".to_vec();
    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(202),
            source_path: "/repo/fresh.rs".to_string(),
            source_bytes: source_bytes.clone(),
            source_blob: None,
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
        None,
    )
    .await;

    assert!(result, "fresh ParseArtifact should succeed");

    // First input must be ParserStarted with correct blob identity.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserStarted(ps))) => {
            assert_eq!(ps.artifact_id, ArtifactId::new(202));
            assert_eq!(ps.source_path, "/repo/fresh.rs");
            assert_eq!(ps.title, "fresh-artifact");
            assert_eq!(ps.content_hash, content_hash(&source_bytes));
            // Verify the blob is actually in the store.
            let stored = blob_store.get(ps.blob_id).expect("blob should be in store");
            assert_eq!(stored, source_bytes);
        }
        Ok(Some(other)) => panic!("expected ParserStarted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserStarted"),
        Err(_) => panic!("timeout waiting for ParserStarted"),
    }
}

#[tokio::test]
async fn resume_sends_record_evidence_when_evidence_already_in_state() {
    // When state already contains an evidence record at a deterministic
    // ID (e.g. from a prior crashed parse), the runtime must still send
    // RecordEvidence so the domain handler can repair/replace a malformed
    // record. The domain handler will no-op valid duplicates internally.
    use maestria_domain::evidence_id_for;
    use maestria_domain::{Evidence, EvidenceKind, LogicalTick};

    let art_id = ArtifactId::new(300);
    let ev_id = evidence_id_for(art_id, 0);

    // Pre-populate the blob store with known bytes.
    let blob_store = Arc::new(InMemoryBlobStore::new());
    let resume_bytes = b"repair evidence test".to_vec();
    let blob_id = blob_store
        .put(resume_bytes.clone())
        .expect("pre-populate blob");

    let artifact_repo = InMemoryArtifactRepository::new();
    artifact_repo
        .put(Artifact {
            id: art_id,
            title: "repair-artifact".into(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Unindexed,
            content_hash: None,
        })
        .expect("pre-populated artifact should be accepted");

    // Pre-populate the event log with ParserStarted so the runtime
    // treats this as a resume path.
    let event_log = Arc::new(InMemoryEventLog::new());
    event_log
        .append(DomainEventEnvelope {
            id: maestria_domain::EventId::new(1),
            sequence: maestria_domain::SequenceNumber::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id: art_id,
                title: "repair-artifact".to_string(),
                source_path: "/repo/repair.rs".to_string(),
                content_hash: maestria_domain::content_hash(&resume_bytes),
                blob_id,
            },
        })
        .expect("append ParserStarted event");

    // Pre-populate runtime state with an existing (malformed) evidence record.
    let mut initial_state = KernelState::new();
    initial_state.evidences.insert(
        ev_id,
        Evidence {
            id: ev_id,
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: maestria_domain::HarnessRunId::new(1),
                stream: maestria_domain::OutputStream::Stdout,
                blob: BlobId::new(99),
            },
            excerpt: "stale".to_string(),
            observed_at: LogicalTick::new(1),
        },
    );

    let adapters = Adapters {
        event_log,
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
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: art_id,
            source_path: "/repo/repair.rs".to_string(),
            source_bytes: Vec::new(), // empty — bytes come from blob store
            source_blob: Some(blob_id),
        }),
        Arc::new(adapters),
        Arc::new(governance),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(initial_state)),
        input_tx,
        None,
    )
    .await;

    assert!(result, "resume ParseArtifact should succeed");

    // Resume must NOT send ParserStarted (already in log).
    // First input should be ParserCompleted.
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::ParserCompleted(pr))) => {
            assert_eq!(pr.artifact_id, art_id);
            assert_eq!(pr.chunks.len(), 1);
            assert_eq!(pr.chunks[0].text, "repair evidence test");
        }
        Ok(Some(DomainInput::ParserStarted(_))) => {
            panic!("resume must not send ParserStarted again");
        }
        Ok(Some(other)) => panic!("expected ParserCompleted, got {other:?}"),
        Ok(None) => panic!("channel closed before ParserCompleted"),
        Err(_) => panic!("timeout waiting for ParserCompleted"),
    }

    // Must still send RecordEvidence for the chunk despite evidence
    // already existing in state (so the domain handler can repair).
    match tokio::time::timeout(Duration::from_secs(1), input_rx.recv()).await {
        Ok(Some(DomainInput::RecordEvidence(ev))) => {
            assert_eq!(ev.evidence_id, ev_id);
            assert_eq!(ev.artifact_id, art_id);
            assert_eq!(ev.claim_id, None);
            match &ev.kind {
                EvidenceKind::FileSpan { snapshot, .. } => {
                    assert!(snapshot.is_some(), "evidence must carry a blob snapshot");
                }
                _ => panic!("expected FileSpan evidence, got {:?}", ev.kind),
            }
        }
        Ok(Some(other)) => panic!("expected RecordEvidence, got {other:?}"),
        Ok(None) => panic!("channel closed before RecordEvidence"),
        Err(_) => panic!("timeout waiting for RecordEvidence"),
    }
}
