use super::*;
use crate::schema::{CURRENT_SCHEMA_VERSION, migrate};
use maestria_domain::*;
use maestria_ports::contract_tests;
use maestria_ports::*;
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

#[test]
fn satisfies_shared_artifact_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_artifact_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_event_log_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_event_log_round_trip(&store);
}

#[test]
fn satisfies_shared_chunk_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_chunk_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_card_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_card_repository_round_trip(&store);
}

#[test]
fn satisfies_shared_evidence_repository_contract() {
    let store = SqliteStore::in_memory().expect("test setup");

    contract_tests::assert_evidence_repository_round_trip(&store);
}

fn artifact(id: u64) -> Artifact {
    Artifact {
        id: ArtifactId::new(id),
        title: format!("artifact {id}"),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::default(),
        content_hash: None,
    }
}

fn registered(event_id: u64, sequence: u64, artifact_id: u64) -> DomainEventEnvelope {
    DomainEventEnvelope {
        id: EventId::new(event_id),
        sequence: SequenceNumber::new(sequence),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(artifact_id),
            title: format!("artifact {artifact_id}"),
        },
    }
}

#[test]
fn migrations_are_idempotent() {
    let directory = tempfile::tempdir().expect("test setup");
    let path = directory.path().join("store.db");

    SqliteStore::open(&path).expect("test setup");
    SqliteStore::open(&path).expect("test setup");

    let connection = Connection::open(path).expect("test setup");
    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .expect("test setup");
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    for table in ["chunks", "cards", "card_claims", "evidence"] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(count, 1, "{table} table should exist");
    }
}

#[test]
fn artifact_put_get_and_missing() {
    let store = SqliteStore::in_memory().expect("test setup");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(9)).expect("missing artifact lookup"),
        None
    );

    let artifact = artifact(1);
    ArtifactRepository::put(&store, artifact.clone()).expect("test setup");

    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("stored artifact lookup"),
        Some(artifact)
    );
}

#[test]
fn artifact_relationship_sets_round_trip() {
    let store = SqliteStore::in_memory().expect("test setup");
    let mut artifact = artifact(1);
    artifact
        .chunk_ids
        .extend([ChunkId::new(10), ChunkId::new(11)]);
    artifact.card_ids.extend([CardId::new(20), CardId::new(21)]);
    artifact
        .claim_ids
        .extend([ClaimId::new(30), ClaimId::new(31)]);
    artifact
        .evidence_ids
        .extend([EvidenceId::new(40), EvidenceId::new(41)]);

    ArtifactRepository::put(&store, artifact.clone()).expect("test setup");

    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("stored artifact lookup"),
        Some(artifact)
    );
}

#[test]
fn brain_state_round_trips_and_lists_deterministically() {
    let store = SqliteStore::in_memory().expect("test setup");
    let late = Chunk {
        id: ChunkId::new(10),
        artifact_id: ArtifactId::new(1),
        order: 2,
        text: "late".to_string(),
    };
    let early = Chunk {
        id: ChunkId::new(11),
        artifact_id: ArtifactId::new(1),
        order: 1,
        text: "early".to_string(),
    };
    let card = Card {
        id: CardId::new(20),
        artifact_id: ArtifactId::new(1),
        title: "card".to_string(),
        body: "body".to_string(),
        claim_ids: [ClaimId::new(5), ClaimId::new(3)].into(),
    };
    let evidence = Evidence {
        id: EvidenceId::new(30),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(3)),
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_string(),
            range: ContentRange { start: 3, end: 8 },
            content_hash: "sha256:abc".to_string(),
            snapshot: None,
        },
        excerpt: "grounded excerpt".to_string(),
        observed_at: LogicalTick::new(4),
    };

    ChunkRepository::put(&store, late.clone()).expect("late chunk put");
    ChunkRepository::put(&store, early.clone()).expect("early chunk put");
    CardRepository::put(&store, card.clone()).expect("card put");
    EvidenceRepository::put(&store, evidence.clone()).expect("evidence put");

    assert_eq!(
        ChunkRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("chunk list"),
        vec![early.clone(), late.clone()]
    );
    assert_eq!(
        ChunkRepository::get(&store, late.id).expect("chunk get"),
        Some(late)
    );
    assert_eq!(
        CardRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("card list"),
        vec![card.clone()]
    );
    assert_eq!(
        CardRepository::get(&store, card.id).expect("card get"),
        Some(card)
    );
    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("evidence list"),
        vec![evidence.clone()]
    );
    assert_eq!(
        EvidenceRepository::get(&store, evidence.id).expect("evidence get"),
        Some(evidence)
    );
}

#[test]
fn evidence_kind_persists_without_domain_serde() {
    let store = SqliteStore::in_memory().expect("test setup");
    let command = Evidence {
        id: EvidenceId::new(31),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::CommandOutput {
            harness_run: HarnessRunId::new(8),
            stream: OutputStream::Stderr,
            blob: BlobId::new(9),
        },
        excerpt: "stderr excerpt".to_string(),
        observed_at: LogicalTick::new(5),
    };
    let validation = Evidence {
        id: EvidenceId::new(32),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(10),
        },
        excerpt: "validation excerpt".to_string(),
        observed_at: LogicalTick::new(6),
    };

    EvidenceRepository::put(&store, command.clone()).expect("command evidence put");
    EvidenceRepository::put(&store, validation.clone()).expect("validation evidence put");

    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("evidence list"),
        vec![command, validation]
    );
}

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
fn migration_rejects_event_metadata_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("mismatched-metadata.db");
    {
        let connection = Connection::open(&path)?;
        let payload = r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"artifact"}"#;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             CREATE TABLE domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL
             );",
        )?;
        connection.execute("INSERT INTO schema_version (version) VALUES (?1);", [2])?;
        connection.execute(
            "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', NULL, ?1, 2)",
            [payload],
        )?;
    }

    assert!(matches!(
        SqliteStore::open(&path),
        Err(PortError::Internal { .. })
    ));
    Ok(())
}

#[test]
fn legacy_migration_rejects_lossy_existing_payloads() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lossy-legacy.db");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)
                 VALUES (1, 1, 'chunk_registered', 1, '{\"event_kind\":\"chunk_registered\",\"chunk_id\":1,\"artifact_id\":1,\"order\":0}');",
        )?;
    }

    assert!(matches!(
        SqliteStore::open(&path),
        Err(PortError::InvalidInput { .. })
    ));
    Ok(())
}

#[test]
fn legacy_event_rows_migrate_and_reject_lossy_payloads() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("legacy.db");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );",
        )?;
    }

    let store = SqliteStore::open(&path)?;
    {
        let connection = store.lock()?;
        connection.execute(
            "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', 1, ?1, 1)",
            params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"legacy"}"#],
        )?;
    }
    assert_eq!(
        store.scan(EventFilter { artifact_id: None })?,
        vec![DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(1),
                title: "legacy".to_string(),
            },
        }]
    );

    {
        let connection = store.lock()?;
        connection.execute(
            "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (2, 2, 'relation_created', NULL, ?1, 1)",
            params![r#"{"event_kind":"relation_created","relation_id":7}"#],
        )?;
    }
    assert!(matches!(
        store.scan(EventFilter { artifact_id: None }),
        Err(PortError::InvalidInput { .. })
    ));

    let connection = store.lock()?;
    let has_payload_version: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('domain_events') WHERE name = 'payload_version'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(has_payload_version, 1);
    Ok(())
}

#[test]
fn migration_rejects_non_nullable_artifact_column() -> Result<(), PortError> {
    let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (2);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER NOT NULL,
                     payload_json TEXT NOT NULL,
                     payload_version INTEGER NOT NULL
                 );",
        )
        .map_err(to_port_error)?;

    assert!(matches!(
        migrate(&mut connection),
        Err(PortError::Internal { message }) if message.contains("artifact_id")
    ));
    Ok(())
}

#[test]
fn legacy_migration_rejects_noncontiguous_event_rows() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed-legacy.db");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)
                 VALUES (9, 9, 'artifact_registered', 1, '{\"event_kind\":\"artifact_registered\",\"artifact_id\":1,\"title\":\"legacy\"}');",
        )?;
    }

    assert!(matches!(
        SqliteStore::open(&path),
        Err(PortError::Internal { .. })
    ));
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
fn evidence_put_is_idempotent() {
    let store = SqliteStore::in_memory().expect("test setup");
    let evidence = Evidence {
        id: EvidenceId::new(100),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "test excerpt".to_string(),
        observed_at: LogicalTick::new(1),
    };
    EvidenceRepository::put(&store, evidence.clone()).expect("first put must succeed");
    EvidenceRepository::put(&store, evidence.clone()).expect("identical retry must succeed");
    let stored = EvidenceRepository::get(&store, evidence.id)
        .expect("get after retry")
        .expect("evidence must still exist");
    assert_eq!(stored, evidence);
}

#[test]
fn evidence_put_rejects_conflicting_overwrite() {
    let store = SqliteStore::in_memory().expect("test setup");
    let first = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "original".to_string(),
        observed_at: LogicalTick::new(1),
    };
    EvidenceRepository::put(&store, first.clone()).expect("first put must succeed");

    let conflict = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "different".to_string(),
        observed_at: LogicalTick::new(2),
    };
    let err = EvidenceRepository::put(&store, conflict).unwrap_err();
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "conflicting put must return Conflict, got {err:?}"
    );

    let stored = EvidenceRepository::get(&store, first.id)
        .expect("get after conflict")
        .expect("evidence must still exist");
    assert_eq!(stored, first);
}

#[test]
fn pending_index_event_round_trips() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(7),
            content_hash: "sha256:abc123".to_string(),
        },
    };
    store.append(event.clone()).expect("append");
    let scanned = store
        .scan(EventFilter { artifact_id: None })
        .expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn full_text_indexed_event_round_trips() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::FullTextIndexed {
            artifact_id: ArtifactId::new(7),
            chunk_id: ChunkId::new(42),
        },
    };
    store.append(event.clone()).expect("append");
    let scanned = store
        .scan(EventFilter { artifact_id: None })
        .expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn artifact_indexed_event_round_trips() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(7),
        },
    };
    store.append(event.clone()).expect("append");
    let scanned = store
        .scan(EventFilter { artifact_id: None })
        .expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn index_events_filter_by_artifact() {
    let store = SqliteStore::in_memory().expect("test setup");
    let pending = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(1),
            content_hash: "sha256:aaa".to_string(),
        },
    };
    let full_text = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::FullTextIndexed {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    };
    let indexed = DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(1),
        },
    };
    let other = DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(2),
        },
    };
    store.append(pending.clone()).expect("append");
    store.append(full_text.clone()).expect("append");
    store.append(indexed.clone()).expect("append");
    store.append(other.clone()).expect("append");

    let for_artifact_1 = store
        .scan(EventFilter {
            artifact_id: Some(ArtifactId::new(1)),
        })
        .expect("filtered scan");
    assert_eq!(for_artifact_1, vec![pending, full_text, indexed]);
}

#[test]
fn artifact_index_status_round_trips_through_repository() {
    let store = SqliteStore::in_memory().expect("test setup");

    // Default artifact has Unindexed status and no content_hash
    let mut artifact = artifact(1);
    assert_eq!(artifact.index_status, IndexStatus::Unindexed);
    assert_eq!(artifact.content_hash, None);
    ArtifactRepository::put(&store, artifact.clone()).expect("put");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("get"),
        Some(artifact.clone())
    );

    // Update to Pending with a content_hash
    artifact.index_status = IndexStatus::Pending;
    artifact.content_hash = Some("sha256:def456".to_string());
    ArtifactRepository::put(&store, artifact.clone()).expect("put");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("get"),
        Some(artifact.clone())
    );

    // Update to Indexed, keep content_hash
    artifact.index_status = IndexStatus::Indexed;
    ArtifactRepository::put(&store, artifact.clone()).expect("put");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("get"),
        Some(artifact)
    );
}
