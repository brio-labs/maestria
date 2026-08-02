use super::super::contract_tests::*;
use super::super::*;
use maestria_domain::{ArtifactId, BlobId, ChunkId, ContentHash, DomainEvent, DomainEventEnvelope};

fn hash_a() -> Result<ContentHash, PortError> {
    ContentHash::new(format!("sha256:{:064x}", 5)).map_err(|error| PortError::InvalidInputContext {
        context: "test hash a",
        source: error.to_string(),
    })
}

fn hash_b() -> Result<ContentHash, PortError> {
    ContentHash::new(format!("sha256:{:064x}", 6)).map_err(|error| PortError::InvalidInputContext {
        context: "test hash b",
        source: error.to_string(),
    })
}

#[test]
fn in_memory_event_log_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_event_log_round_trip(&InMemoryEventLog::new())?;
    Ok(())
}

#[test]
fn in_memory_event_log_filters_task_artifact_events() -> Result<(), PortError> {
    let log = InMemoryEventLog::new();
    let task = DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::TaskOpened {
            task_id: maestria_domain::TaskId::new(1),
            title: "task".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            artifact_id: Some(maestria_domain::ArtifactId::new(7)),
        },
    };
    log.append(task.clone())?;
    assert_eq!(
        log.scan(EventFilter {
            artifact_id: Some(maestria_domain::ArtifactId::new(7)),
        })?,
        vec![task]
    );
    Ok(())
}

#[test]
fn in_memory_event_log_roundtrips_search_executed() -> Result<(), PortError> {
    let log = InMemoryEventLog::new();
    let envelope = DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::SearchExecuted {
            query: "audit".to_string(),
            limit: 3,
            evidence_ids: vec![maestria_domain::EvidenceId::new(5)],
            pack_metadata: None,
            at: maestria_domain::LogicalTick::new(2),
        },
    };
    log.append(envelope.clone())?;
    // Full scan must return the event.
    assert_eq!(
        log.scan(EventFilter { artifact_id: None })?,
        vec![envelope.clone()]
    );
    // Artifact-filtered scan must exclude SearchExecuted (no artifact_id field).
    assert!(
        log.scan(EventFilter {
            artifact_id: Some(maestria_domain::ArtifactId::new(1)),
        })?
        .is_empty()
    );
    Ok(())
}

fn artifact_filter_events(
    artifact_a: ArtifactId,
    artifact_b: ArtifactId,
) -> Result<Vec<DomainEventEnvelope>, PortError> {
    Ok(vec![
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(1),
            sequence: maestria_domain::SequenceNumber::new(1),
            event: DomainEvent::PendingIndex {
                artifact_id: artifact_a,
                content_hash: hash_a()?,
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(2),
            sequence: maestria_domain::SequenceNumber::new(2),
            event: DomainEvent::FullTextIndexed {
                artifact_id: artifact_a,
                chunk_id: ChunkId::new(1),
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(3),
            sequence: maestria_domain::SequenceNumber::new(3),
            event: DomainEvent::ArtifactIndexed {
                artifact_id: artifact_a,
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(4),
            sequence: maestria_domain::SequenceNumber::new(4),
            event: DomainEvent::ParserStarted {
                artifact_id: artifact_a,
                title: "doc".to_string(),
                source_path: "/a.md".to_string(),
                content_hash: hash_a()?,
                blob_id: BlobId::new(1),
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(5),
            sequence: maestria_domain::SequenceNumber::new(5),
            event: DomainEvent::SourceBecameStale {
                artifact_id: artifact_a,
                source_path: "/a.md".to_string(),
                content_hash: hash_a()?,
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(6),
            sequence: maestria_domain::SequenceNumber::new(6),
            event: DomainEvent::PendingIndex {
                artifact_id: artifact_b,
                content_hash: hash_b()?,
            },
        },
    ])
}

#[test]
fn in_memory_event_log_filters_all_artifact_variants() -> Result<(), PortError> {
    let log = InMemoryEventLog::new();
    let artifact_a = ArtifactId::new(1);
    let artifact_b = ArtifactId::new(2);
    let events = artifact_filter_events(artifact_a, artifact_b)?;

    for event in &events {
        log.append(event.clone())?;
    }

    assert_eq!(
        log.scan(EventFilter { artifact_id: None })?,
        events,
        "global scan should return all events"
    );

    let filtered_a = log.scan(EventFilter {
        artifact_id: Some(artifact_a),
    })?;
    assert_eq!(filtered_a.len(), 5);
    for event in &filtered_a {
        match &event.event {
            DomainEvent::PendingIndex { artifact_id, .. }
            | DomainEvent::FullTextIndexed { artifact_id, .. }
            | DomainEvent::ArtifactIndexed { artifact_id }
            | DomainEvent::ParserStarted { artifact_id, .. }
            | DomainEvent::SourceBecameStale { artifact_id, .. } => {
                assert_eq!(*artifact_id, artifact_a);
            }
            other => {
                return Err(PortError::InternalContext {
                    context: "unexpected artifact-filtered event variant",
                    source: format!("{other:?}"),
                });
            }
        }
    }

    let filtered_b = log.scan(EventFilter {
        artifact_id: Some(artifact_b),
    })?;
    assert_eq!(filtered_b.len(), 1);
    assert!(matches!(
        filtered_b[0].event,
        DomainEvent::PendingIndex { artifact_id, .. } if artifact_id == artifact_b
    ));
    assert!(
        log.scan(EventFilter {
            artifact_id: Some(ArtifactId::new(99)),
        })?
        .is_empty()
    );
    Ok(())
}
