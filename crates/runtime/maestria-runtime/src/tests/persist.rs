use crate::test_support::*;
use crate::tests::FailingEventLog;
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, DomainEvent, DomainEventEnvelope, EventId,
    Evidence, EvidenceId, EvidenceKind, IndexStatus, LogicalTick, SequenceNumber, SourceSpan,
    StructureNodeId,
};
use maestria_ports::{
    CardRepository, ChunkRepository, EventFilter, EventLog, EvidenceRepository,
    InMemoryArtifactRepository, InMemoryCardRepository, InMemoryChunkRepository, InMemoryEventLog,
    InMemoryEvidenceRepository,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

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
