use crate::{SqliteStore, to_port_error};
use maestria_domain::*;
use maestria_ports::*;
use rusqlite::params;

use super::registered;

#[test]
fn event_append_scan_order_and_filter() {
    let store = SqliteStore::in_memory().expect("test setup");
    let first = registered(1, 1, 7);
    let second = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::TaskOpened {
            task_id: TaskId::new(99),
            title: "task".to_string(),
            priority: TaskPriority::High,
            artifact_id: Some(ArtifactId::new(7)),
        },
    };
    let third = DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(8),
            artifact_id: ArtifactId::new(7),
            order: 0,
            text: "chunk".to_string(),
            node_id: maestria_domain::StructureNodeId::new(0),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 2,
            },
            representations: vec![],
        },
    };
    let out_of_order = DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(1),
        },
    };

    store.append(first.clone()).expect("test setup");
    store.append(second.clone()).expect("test setup");
    store.append(third.clone()).expect("test setup");
    assert!(matches!(
        store.append(out_of_order),
        Err(PortError::Conflict { .. })
    ));

    assert_eq!(
        store
            .scan(EventFilter { artifact_id: None })
            .expect("full event scan"),
        vec![first.clone(), second.clone(), third.clone()]
    );
    assert_eq!(
        store
            .scan(EventFilter {
                artifact_id: Some(ArtifactId::new(7)),
            })
            .expect("filtered event scan"),
        vec![first, second, third]
    );
}

#[test]
fn artifact_filter_includes_evidence_and_search_events() {
    let store = SqliteStore::in_memory().expect("test setup");
    let evidence = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::EvidenceRecorded {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(7),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range: ContentRange { start: 1, end: 4 },
                content_hash: "sha256:notes".to_string(),
                snapshot: None,
            },
            excerpt: "excerpt".to_string(),
            observed_at: LogicalTick::new(1),
        },
    };
    let search = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::SearchCompleted {
            artifact_id: ArtifactId::new(7),
            cards_added: 2,
        },
    };
    let unrelated = registered(3, 3, 9);

    store.append(evidence.clone()).expect("evidence append");
    store.append(search.clone()).expect("search append");
    store.append(unrelated).expect("unrelated append");

    assert_eq!(
        store
            .scan(EventFilter {
                artifact_id: Some(ArtifactId::new(7)),
            })
            .expect("filtered event scan"),
        vec![evidence, search]
    );
}

#[test]
fn duplicate_event_id_or_sequence_conflicts() {
    let store = SqliteStore::in_memory().expect("test setup");
    store.append(registered(1, 1, 1)).expect("test setup");

    assert!(matches!(
        store.append(registered(1, 2, 1)),
        Err(PortError::Conflict { .. })
    ));
    assert!(matches!(
        store.append(registered(2, 1, 1)),
        Err(PortError::Conflict { .. })
    ));
}

#[test]
fn append_rejects_swapped_existing_event_rows() -> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    {
        let connection = store.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 2, 'artifact_registered', 1, ?1, 2)",
                params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"first"}"#],
            )
            .map_err(to_port_error)?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (2, 1, 'artifact_registered', 1, ?1, 2)",
                params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"second"}"#],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        store.append(registered(3, 3, 1)),
        Err(PortError::Conflict { .. })
    ));
    Ok(())
}

#[test]
fn fresh_schema_writes_payload_version_two() -> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    store.append(registered(1, 1, 1))?;
    let connection = store.lock()?;
    let version: i64 = connection
        .query_row(
            "SELECT payload_version FROM domain_events WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    assert_eq!(version, 2);
    Ok(())
}

#[test]
fn malformed_v2_payload_is_rejected_without_defaults() -> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    {
        let connection = store.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'chunk_registered', 1, ?1, 2)",
                params![r#"{"event_kind":"chunk_registered","chunk_id":1,"artifact_id":1,"order":0}"#],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        store.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));
    Ok(())
}

#[test]
fn strict_v2_payloads_reject_missing_and_unknown_fields() -> Result<(), PortError> {
    let missing_warnings = SqliteStore::in_memory()?;
    {
        let connection = missing_warnings.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'validation_report_created', NULL, ?1, 2)",
                params![
                    r#"{"event_kind":"validation_report_created","report_id":1,"task_id":null,"passed":true}"#
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        missing_warnings.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));

    let unknown_field = SqliteStore::in_memory()?;
    {
        let connection = unknown_field.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'artifact_registered', 1, ?1, 2)",
                params![
                    r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"artifact","unexpected":true}"#
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        unknown_field.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));
    let unknown_nested_field = SqliteStore::in_memory()?;
    {
        let connection = unknown_nested_field.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'relation_created', NULL, ?1, 2)",
                params![
                    r#"{"event_kind":"relation_created","relation_id":1,"source":{"kind":"artifact","artifact_id":1,"unexpected":true},"kind":"supports","target":{"kind":"artifact","artifact_id":2},"evidence_id":null,"confidence_milli":1000}"#
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        unknown_nested_field.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));

    let mismatched_metadata = SqliteStore::in_memory()?;
    {
        let connection = mismatched_metadata.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'artifact_registered', NULL, ?1, 2)",
                params![
                    r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"artifact"}"#
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        mismatched_metadata.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));

    Ok(())
}

#[test]
fn task_evidence_linked_event_round_trips() -> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    let envelope = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::TaskEvidenceLinked {
            task_id: TaskId::new(3),
            evidence_id: EvidenceId::new(10),
        },
    };

    store.append(envelope.clone())?;

    let events = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], envelope);

    Ok(())
}

#[test]
fn search_executed_roundtrips_through_appended_scan() {
    let store = SqliteStore::in_memory().expect("test setup");
    let envelope = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::SearchExecuted {
            query: "test query".to_string(),
            limit: 5,
            evidence_ids: vec![EvidenceId::new(10), EvidenceId::new(20)],
            at: LogicalTick::new(3),
        },
    };
    store
        .append(envelope.clone())
        .expect("append search executed");
    let scanned = store
        .scan(EventFilter { artifact_id: None })
        .expect("scan events");
    assert_eq!(scanned, vec![envelope]);
}

#[test]
fn document_tree_captured_event_round_trips() -> Result<(), PortError> {
    let store = SqliteStore::in_memory()?;
    let node = StructureNode {
        id: StructureNodeId::new(42),
        parent_id: None,
        sibling_id: None,
        node_type: maestria_domain::StructureNodeType::Document,
        source_range: ContentRange { start: 0, end: 100 },
        page: Some(1),
        section_path: vec!["Intro".to_string()],
        parser_generation: "v1".to_string(),
        schema_generation: "v2".to_string(),
        language: Some("en".to_string()),
    };

    let envelope = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::DocumentTreeCaptured {
            artifact_id: ArtifactId::new(3),
            artifact_version_id: ArtifactVersionId::new(5),
            content_hash: ContentHash::new(format!("sha256:{}", "a".repeat(64)))
                .expect("test hash is valid"),
            root_id: StructureNodeId::new(42),
            nodes: vec![node],
        },
    };

    store.append(envelope.clone())?;

    let events = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], envelope);

    Ok(())
}
