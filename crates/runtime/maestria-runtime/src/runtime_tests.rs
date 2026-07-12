use super::*;
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, DomainEventEnvelope, EventId,
    Evidence, EvidenceId, EvidenceKind, IndexStatus, LogicalTick, SequenceNumber,
};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier};
use maestria_ports::{
    CardRepository, ChunkRepository, EventFilter, EventLog, EvidenceRepository,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository,
    InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryParser,
    InMemoryVectorIndex, InMemoryWebFetcher, PortError,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct FailingEventLog;

impl EventLog for FailingEventLog {
    fn append(&self, _event: maestria_domain::DomainEventEnvelope) -> Result<(), PortError> {
        Err(PortError::Downstream {
            message: "event store unavailable".to_string(),
        })
    }

    fn scan(
        &self,
        _filter: EventFilter,
    ) -> Result<Vec<maestria_domain::DomainEventEnvelope>, PortError> {
        Ok(Vec::new())
    }
}


#[tokio::test]
async fn persist_effects_keep_duplicate_events_in_order() {
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        blob_store: Arc::new(InMemoryBlobStore::new()),
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
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 2,
            default_effect_timeout: Duration::from_secs(2),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        governance,
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    input_tx
        .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(7)))
        .await
        .expect("first tick should be accepted");
    input_tx
        .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(7)))
        .await
        .expect("second tick should be accepted");

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if event_log
                .scan(EventFilter { artifact_id: None })
                .expect("event scan")
                .len()
                == 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("persist effects should complete");

    let events = event_log
        .scan(EventFilter { artifact_id: None })
        .expect("event scan");
    assert_eq!(events[0].id.value(), 1);
    assert_eq!(events[0].sequence.value(), 1);
    assert_eq!(events[1].id.value(), 2);
    assert_eq!(events[1].sequence.value(), 2);
    assert_eq!(events[0].event, events[1].event);

    shutdown.cancel();
    run.await.expect("runtime should shut down");
}

#[tokio::test]
async fn failed_event_persistence_stops_runtime() {
    let adapters = Adapters {
        event_log: Arc::new(FailingEventLog),
        blob_store: Arc::new(InMemoryBlobStore::new()),
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
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            default_effect_timeout: Duration::from_secs(1),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        governance,
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    input_tx
        .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(1)))
        .await
        .expect("tick should be accepted before persistence failure");

    tokio::time::timeout(Duration::from_secs(2), run)
        .await
        .expect("runtime should stop after fatal persistence failure")
        .expect("runtime task should join");
    assert!(shutdown.is_cancelled());
}

#[tokio::test]
async fn persist_event_dispatches_chunk_card_evidence_to_repositories() {
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(1);
    let card_id = CardId::new(1);
    let evidence_id = EvidenceId::new(1);

    let artifact = Artifact {
        id: artifact_id,
        title: "test".into(),
        chunk_ids: [chunk_id].into(),
        card_ids: [card_id].into(),
        claim_ids: BTreeSet::new(),
        evidence_ids: [evidence_id].into(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
    };
    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        order: 0,
        text: "chunk text".into(),
    };
    let card = Card {
        id: card_id,
        artifact_id,
        title: "card title".into(),
        body: "card body".into(),
        claim_ids: BTreeSet::new(),
    };
    let evidence = Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/test.txt".into(),
            range: maestria_domain::ContentRange { start: 0, end: 10 },
            content_hash: "abc".into(),
            snapshot: None,
        },
        excerpt: "excerpt".into(),
        observed_at: LogicalTick::new(1),
    };

    let mut state = KernelState::new();
    state.artifacts.insert(artifact_id, artifact);
    state.chunks.insert(chunk_id, chunk);
    state.cards.insert(card_id, card);
    state.evidences.insert(evidence_id, evidence);

    let chunk_repo = Arc::new(InMemoryChunkRepository::new());
    let card_repo = Arc::new(InMemoryCardRepository::new());
    let evidence_repo = Arc::new(InMemoryEvidenceRepository::new());
    let artifact_repo = Arc::new(InMemoryArtifactRepository::new());
    let event_log = Arc::new(InMemoryEventLog::new());

    let adapters = Adapters {
        event_log: event_log.clone(),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: artifact_repo.clone(),
        chunk_repo: chunk_repo.clone(),
        card_repo: card_repo.clone(),
        evidence_repo: evidence_repo.clone(),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    };
    let governance = Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    };
    let (input_tx, _input_rx) = mpsc::channel(8);

    let envelopes = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                order: 0,
                text: "chunk text".into(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::CardCreated {
                card_id,
                artifact_id,
                title: "card title".into(),
                body: "card body".into(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/test.txt".into(),
                    range: maestria_domain::ContentRange { start: 0, end: 10 },
                    content_hash: "abc".into(),
                    snapshot: None,
                },
                excerpt: "excerpt".into(),
                observed_at: LogicalTick::new(1),
            },
        },
    ];

    let adapters = Arc::new(adapters);
    let governance = Arc::new(governance);

    for envelope in &envelopes {
        let result = MaestriaRuntime::execute_effect(
            MaestriaEffect::PersistEvent {
                envelope: envelope.clone(),
            },
            adapters.clone(),
            governance.clone(),
            AutonomyProfile::TrustedWorkspace,
            Scope::default(),
            Arc::new(RwLock::new(state.clone())),
            input_tx.clone(),
        )
        .await;
        assert!(result, "persist of {:?} should succeed", envelope.event);
    }

    assert!(
        chunk_repo.get(chunk_id).is_ok_and(|value| value.is_some()),
        "chunk should be persisted"
    );
    assert!(
        card_repo.get(card_id).is_ok_and(|value| value.is_some()),
        "card should be persisted"
    );
    assert!(
        evidence_repo
            .get(evidence_id)
            .is_ok_and(|value| value.is_some()),
        "evidence should be persisted"
    );
}

