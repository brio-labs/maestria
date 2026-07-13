use super::EffectExecutionContext;
use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Evidence, EvidenceId, EvidenceKind, IndexStatus, LogicalTick,
    ParseArtifactRequest,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier, DefaultValidationGate};
use maestria_ports::{
    ArtifactRepository, BlobStore, EventLog, InMemoryApprovalRepository,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository, InMemoryChunkRepository,
    InMemoryEffectJournal, InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex,
    InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryIdAllocator, InMemoryParser,
    InMemoryVectorIndex, InMemoryWebFetcher,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
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
        id_allocator: Arc::new(InMemoryIdAllocator::new()),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        approval_repo: Arc::new(InMemoryApprovalRepository::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(200),
            source_path: "/repo/resume.rs".to_string(),
            source_bytes: Vec::new(), // empty — bytes come from blob store
            source_blob: Some(blob_id),
        }),
        ctx,
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
        id_allocator: Arc::new(InMemoryIdAllocator::new()),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        approval_repo: Arc::new(InMemoryApprovalRepository::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(201),
            source_path: "/repo/missing.rs".to_string(),
            source_bytes: Vec::new(),
            source_blob: Some(blob_id),
        }),
        ctx,
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
        id_allocator: Arc::new(InMemoryIdAllocator::new()),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        approval_repo: Arc::new(InMemoryApprovalRepository::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let source_bytes = b"fresh blob identity test".to_vec();
    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: ArtifactId::new(202),
            source_path: "/repo/fresh.rs".to_string(),
            source_bytes: source_bytes.clone(),
            source_blob: None,
        }),
        ctx,
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

/// Pre-populates the event log with ParserStarted and the kernel state with a
/// malformed evidence record, as if a prior parse had crashed after persisting
/// the event but before finalizing evidence.
fn populate_resume_event_log_and_state(
    event_log: &Arc<InMemoryEventLog>,
    state: &mut KernelState,
    art_id: ArtifactId,
    ev_id: EvidenceId,
    blob_id: BlobId,
    resume_bytes: &[u8],
) {
    event_log
        .append(DomainEventEnvelope {
            id: maestria_domain::EventId::new(1),
            sequence: maestria_domain::SequenceNumber::new(1),
            event: DomainEvent::ParserStarted {
                artifact_id: art_id,
                title: "repair-artifact".to_string(),
                source_path: "/repo/repair.rs".to_string(),
                content_hash: maestria_domain::content_hash(resume_bytes),
                blob_id,
            },
        })
        .expect("append ParserStarted event");
    state.evidences.insert(
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
}

async fn assert_parser_completed_for_resume(
    input_rx: &mut mpsc::Receiver<DomainInput>,
    art_id: ArtifactId,
) {
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
}

async fn assert_record_evidence_for_repair(
    input_rx: &mut mpsc::Receiver<DomainInput>,
    ev_id: EvidenceId,
    art_id: ArtifactId,
) {
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
#[tokio::test]
async fn resume_sends_record_evidence_when_evidence_already_in_state() {
    // When state already contains an evidence record at a deterministic
    // ID (e.g. from a prior crashed parse), the runtime must still send
    // RecordEvidence so the domain handler can repair/replace a malformed
    // record. The domain handler will no-op valid duplicates internally.
    use maestria_domain::evidence_id_for;

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

    let event_log = Arc::new(InMemoryEventLog::new());
    let mut initial_state = KernelState::new();
    populate_resume_event_log_and_state(
        &event_log,
        &mut initial_state,
        art_id,
        ev_id,
        blob_id,
        &resume_bytes,
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
        id_allocator: Arc::new(InMemoryIdAllocator::new()),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        approval_repo: Arc::new(InMemoryApprovalRepository::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
        validation_gate: Arc::new(DefaultValidationGate::new(true)),
    };
    let (input_tx, mut input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(initial_state)),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::ParseArtifact(ParseArtifactRequest {
            artifact_id: art_id,
            source_path: "/repo/repair.rs".to_string(),
            source_bytes: Vec::new(),
            source_blob: Some(blob_id),
        }),
        ctx,
        None,
    )
    .await;

    assert!(result, "resume ParseArtifact should succeed");

    assert_parser_completed_for_resume(&mut input_rx, art_id).await;
    assert_record_evidence_for_repair(&mut input_rx, ev_id, art_id).await;
}
