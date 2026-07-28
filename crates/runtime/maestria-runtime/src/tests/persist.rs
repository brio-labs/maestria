use crate::test_support::*;
use crate::tests::FailingEventLog;
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, DomainEvent, DomainEventEnvelope, EventId,
    Evidence, EvidenceId, EvidenceKind, HarnessRunId, IndexStatus, KernelState, LogicalTick,
    SequenceNumber, SourceSpan, StructureNodeId,
};
use maestria_ports::{
    CardRepository, ChunkRepository, EffectJournal, EffectJournalEntry, EffectJournalIntent,
    EffectJournalStatus, EventFilter, EventLog, InMemoryArtifactRepository, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog, InMemoryEvidenceRepository,
    PortError,
};
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

struct FailOnceEffectJournal {
    inner: Arc<InMemoryEffectJournal>,
    fail_terminal: AtomicBool,
}

impl EffectJournal for FailOnceEffectJournal {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        self.inner.record_intent(intent)
    }

    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.record_started(run_id, generation)
    }

    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        self.inner.claim_feedback(run_id, generation)
    }

    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        if status == EffectJournalStatus::Completed
            && self.fail_terminal.swap(false, Ordering::SeqCst)
        {
            return Err(PortError::InternalContext {
                context: "fail-once journal terminalization",
                source: "injected terminalization failure".to_string(),
            });
        }
        self.inner.record_terminal(run_id, generation, status)
    }

    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        self.inner.scan_in_flight()
    }

    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError> {
        self.inner.is_feedback_accepted(run_id, generation)
    }

    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        self.inner.is_current(run_id, generation)
    }
}

#[tokio::test]
async fn failed_feedback_terminalization_is_retryable() -> Result<(), Box<dyn std::error::Error>> {
    let inner = Arc::new(InMemoryEffectJournal::default());
    let journal = Arc::new(FailOnceEffectJournal {
        inner: inner.clone(),
        fail_terminal: AtomicBool::new(true),
    });
    let run_id = HarnessRunId::new(7);
    let entry = inner.record_intent(EffectJournalIntent {
        run_id,
        task_id: None,
        capability: "shell".to_string(),
        command: "echo test".to_string(),
        scope_id: maestria_domain::ScopeId::new(1),
        requested_generation: None,
    })?;
    inner.record_started(run_id, entry.generation)?;
    inner.claim_feedback(run_id, entry.generation)?;

    let adapters = Adapters {
        effect_journal: journal,
        ..crate::test_helpers::test_adapters()
    };
    let (input_tx, _input_rx) = mpsc::channel(8);
    let context = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(crate::test_helpers::test_governance()),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    context
        .feedback_acks
        .lock()
        .map_err(|_| "feedback acknowledgement lock poisoned")?
        .insert(EventId::new(1), (run_id, entry.generation));
    let envelope = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(1),
        },
    };

    assert!(!context.handle_persist_event(envelope.clone()).await);
    assert!(
        context
            .feedback_acks
            .lock()
            .map_err(|_| "feedback acknowledgement lock poisoned")?
            .contains_key(&EventId::new(1))
    );
    assert!(inner.is_feedback_accepted(run_id, entry.generation)?);

    assert!(context.handle_persist_event(envelope).await);
    assert!(
        !context
            .feedback_acks
            .lock()
            .map_err(|_| "feedback acknowledgement lock poisoned")?
            .contains_key(&EventId::new(1))
    );
    assert!(inner.scan_in_flight()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn persist_effects_keep_duplicate_events_in_order() -> Result<(), Box<dyn std::error::Error>>
{
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
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
        .await?;
    input_tx
        .send(DomainInput::ClockTick(maestria_domain::LogicalTick::new(7)))
        .await?;

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let mut events = Vec::new();
            if let Ok(scanned) = event_log.scan(EventFilter { artifact_id: None }) {
                events = scanned;
            }
            if events.len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;

    let mut events = Vec::new();
    if let Ok(scanned) = event_log.scan(EventFilter { artifact_id: None }) {
        events = scanned;
    }
    assert_eq!(events[0].id.value(), 1);
    assert_eq!(events[0].sequence.value(), 1);
    assert_eq!(events[1].id.value(), 2);
    assert_eq!(events[1].sequence.value(), 2);
    assert_eq!(events[0].event, events[1].event);

    shutdown.cancel();
    run.await?;
    Ok(())
}

#[tokio::test]
async fn failed_event_persistence_stops_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let adapters = Adapters {
        event_log: Arc::new(FailingEventLog),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
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
        .await?;

    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}

/// Builds a KernelState pre-populated with one artifact, chunk, card, and
/// evidence record, returning the state together with the IDs for later
/// assertion.
fn build_persist_test_state() -> (KernelState, ChunkId, CardId, EvidenceId, ArtifactId) {
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
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "chunk text".into(),
    };
    let card = Card {
        id: card_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        title: "card title".into(),
        body: "card body".into(),
        claim_ids: BTreeSet::new(),
        security: maestria_domain::SecurityMetadata::default(),
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
        security: maestria_domain::SecurityMetadata::default(),
    };

    let mut state = KernelState::new();
    state.artifacts.insert(artifact_id, artifact);
    state.chunks.insert(chunk_id, chunk);
    state.cards.insert(card_id, card);
    state.evidences.insert(evidence_id, evidence);
    (state, chunk_id, card_id, evidence_id, artifact_id)
}

fn build_persist_test_envelopes(
    chunk_id: ChunkId,
    card_id: CardId,
    evidence_id: EvidenceId,
    artifact_id: ArtifactId,
) -> Vec<DomainEventEnvelope> {
    vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                node_id: StructureNodeId::new(0),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
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
                node_id: StructureNodeId::new(0),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                title: "card title".into(),
                body: "card body".into(),
                security: maestria_domain::SecurityMetadata::default(),
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
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
    ]
}

#[tokio::test]
async fn persist_event_dispatches_chunk_card_evidence_to_repositories()
-> Result<(), Box<dyn std::error::Error>> {
    let (state, chunk_id, card_id, evidence_id, artifact_id) = build_persist_test_state();

    let chunk_repo = Arc::new(InMemoryChunkRepository::new());
    let card_repo = Arc::new(InMemoryCardRepository::new());
    let evidence_repo = Arc::new(InMemoryEvidenceRepository::new());
    let artifact_repo = Arc::new(InMemoryArtifactRepository::new());
    let event_log = Arc::new(InMemoryEventLog::new());

    let adapters = Adapters {
        event_log: event_log.clone(),
        artifact_repo: artifact_repo.clone(),
        chunk_repo: chunk_repo.clone(),
        card_repo: card_repo.clone(),
        evidence_repo: evidence_repo.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let envelopes = build_persist_test_envelopes(chunk_id, card_id, evidence_id, artifact_id);

    let adapters = Arc::new(adapters);
    let governance = Arc::new(governance);

    for envelope in &envelopes {
        let ctx = EffectExecutionContext::test_default(
            adapters.clone(),
            governance.clone(),
            Arc::new(RwLock::new(state.clone())),
            input_tx.clone(),
        );
        let result = MaestriaRuntime::test_execute_effect(
            MaestriaEffect::PersistEvent {
                envelope: Box::new(envelope.clone()),
            },
            ctx,
            None,
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
    Ok(())
}
