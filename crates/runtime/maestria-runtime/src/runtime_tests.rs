use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, DomainEvent, DomainEventEnvelope, EventId,
    Evidence, EvidenceId, EvidenceKind, FetchWebRequest, FetchWebRequested, IndexStatus,
    LogicalTick, SearchExecutedInput, SequenceNumber, SourceSpan, StructureNodeId,
};
use maestria_governance::Scope;
use maestria_ports::{
    CardRepository, ChunkRepository, EventFilter, EventLog, EvidenceRepository,
    InMemoryArtifactRepository, InMemoryCardRepository, InMemoryChunkRepository, InMemoryEventLog,
    InMemoryEvidenceRepository, PortError,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
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
            let events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_or(Vec::new(), |events| events);
            if events.len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;

    let events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_or(Vec::new(), |events| events);
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

#[tokio::test]
async fn parse_artifact_no_deadlock_at_max_concurrency_one()
-> Result<(), Box<dyn std::error::Error>> {
    use maestria_domain::{ArtifactDetected, content_hash};

    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            default_effect_timeout: Duration::from_secs(5),
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

    let source_bytes = b"fn main() {}".to_vec();
    let source_hash = content_hash(&source_bytes);
    let artifact_id = ArtifactId::new(1);

    // Send ArtifactDetected input — the domain loop produces a
    // ParseArtifact effect, whose handler enqueues ParserStarted and
    // then runs the persistence barrier. With max_concurrent_effects=1,
    // the barrier must not deadlock waiting for the PersistEvent.
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "deadlock-test".to_string(),
            source_path: "/repo/deadlock.rs".to_string(),
            source_bytes,
            content_hash: source_hash,
        }))
        .await?;

    // Wait for the ParserStarted event to be persisted (proves no deadlock).
    let barrier_passed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_or(Vec::new(), |events| events);
            if events.iter().any(|e| {
                matches!(&e.event, DomainEvent::ParserStarted { artifact_id: id, .. } if *id == artifact_id)
            }) {
                break true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(1), run).await;

    let no_deadlock = matches!(barrier_passed, Ok(true));
    assert!(
        no_deadlock,
        "ParserStarted persistence barrier must not deadlock at max_concurrent_effects=1"
    );
    Ok(())
}

// ── SearchExecuted audit event lifecycle ──────────────────────────

#[tokio::test]
async fn search_executed_persists_and_is_observable() -> Result<(), Box<dyn std::error::Error>> {
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
        .send(DomainInput::SearchExecuted(SearchExecutedInput {
            query: "audit test".to_string(),
            limit: 5,
            evidence_ids: vec![EvidenceId::new(10), EvidenceId::new(20)],
            pack_metadata: None,
            at: LogicalTick::new(3),
        }))
        .await?;

    // Wait for the event to appear in the event log.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_or(Vec::new(), |events| events);
            if events
                .iter()
                .any(|env| matches!(env.event, DomainEvent::SearchExecuted { .. }))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;

    let events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_or(Vec::new(), |events| events);
    assert_eq!(events.len(), 1);
    match &events[0].event {
        DomainEvent::SearchExecuted {
            query,
            limit,
            evidence_ids,
            pack_metadata,
            at,
        } => {
            assert_eq!(query, "audit test");
            assert_eq!(*limit, 5);
            assert_eq!(
                evidence_ids,
                &vec![EvidenceId::new(10), EvidenceId::new(20)]
            );
            assert!(pack_metadata.is_none());
            assert_eq!(*at, LogicalTick::new(3));
        }
        _ => return Err("expected SearchExecuted event".to_string().into()),
    }

    shutdown.cancel();
    run.await?;
    Ok(())
}

#[tokio::test]
async fn search_executed_with_failing_event_log_stops_runtime()
-> Result<(), Box<dyn std::error::Error>> {
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
        .send(DomainInput::SearchExecuted(SearchExecutedInput {
            query: "should fail".to_string(),
            limit: 1,
            evidence_ids: vec![],
            pack_metadata: None,
            at: LogicalTick::new(1),
        }))
        .await?;

    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}

// ── spawned executor failure propagation ──────────────────────────────

/// Verify that a failing effect _not_ handled inline (PersistEvent) goes
/// through the spawned executor path and that the runtime is cancelled when
/// the effect fails after all retries are exhausted. This exercises the
/// async supervisor boundary that previously silently discarded
/// EffectFailure values from spawned tasks.
#[tokio::test]
async fn spawned_effect_failure_propagates_to_supervisor_and_cancels_runtime()
-> Result<(), Box<dyn std::error::Error>> {
    // An unseeded InMemoryWebFetcher returns NotFound for any URL that
    // hasn't been seeded, so the FetchWeb effect (non-PersistEvent,
    // always spawned) will fail.
    let adapters = crate::test_helpers::test_adapters();
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            scope: Scope::new(vec![], vec![], vec![], vec![], true),
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

    // Trigger FetchWeb effect via domain input — the domain handler
    // produces a recognised FetchWeb effect, which the executor schedules
    // as a spawned task (not PersistEvent, so the spawned path).
    input_tx
        .send(DomainInput::FetchWebRequested(FetchWebRequested {
            request: FetchWebRequest {
                url: "https://example.com/missing".to_string(),
                max_bytes: 1024,
                max_requests: 1,
                max_latency_ms: 1000,
                allowed_domains: vec![],
                allowed_content_types: vec![],
            },
        }))
        .await?;

    // The runtime should shut down because the spawned FetchWeb effect
    // fails (no seeded URL) and now propagates the failure.
    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}
