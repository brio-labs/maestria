use crate::{SqliteStore, to_port_error};
use maestria_domain::*;
use maestria_ports::*;
use rusqlite::params;

fn make_fingerprint() -> IndexFingerprint {
    IndexFingerprint {
        provider: "openai".to_string(),
        model: "text-embedding-ada-002".to_string(),
        revision: "v1".to_string(),
        artifact_hash: ContentHash::new(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        )
        .expect("generation fixture hash must be valid"),
        dimensions: 1536,
        quantization: "f32".to_string(),
        query_template_hash: "sha256:456".to_string(),
        document_template_hash: "sha256:789".to_string(),
        preprocessing_version: "1.0".to_string(),
    }
}

#[test]
fn index_generation_started_round_trips() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::IndexGenerationStarted {
            id: IndexGenerationId::new(10),
            name: RepresentationName::new("dense_vector"),
            corpus_snapshot: CorpusSnapshotId::new(42),
            fingerprint: make_fingerprint(),
        },
    };
    store.append(event.clone()).expect("append");
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn index_generation_transitioned_round_trips() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::IndexGenerationTransitioned {
            id: IndexGenerationId::new(10),
            from: IndexLifecycle::Building,
            to: IndexLifecycle::Evaluated,
            replaced_active_id: Some(IndexGenerationId::new(5)),
        },
    };
    store.append(event.clone()).expect("append");
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn full_lifecycle_replay_asserts_active_generation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("db.sqlite");

    let fingerprint = make_fingerprint();

    // First session: append the sequence
    {
        let store = SqliteStore::open(&db_path).expect("test setup");

        let started = DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::IndexGenerationStarted {
                id: IndexGenerationId::new(10),
                name: RepresentationName::new("dense_vector"),
                corpus_snapshot: CorpusSnapshotId::new(42),
                fingerprint: fingerprint.clone(),
            },
        };

        let to_evaluated = DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::IndexGenerationTransitioned {
                id: IndexGenerationId::new(10),
                from: IndexLifecycle::Building,
                to: IndexLifecycle::Evaluated,
                replaced_active_id: None,
            },
        };

        let to_shadow = DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::IndexGenerationTransitioned {
                id: IndexGenerationId::new(10),
                from: IndexLifecycle::Evaluated,
                to: IndexLifecycle::Shadow,
                replaced_active_id: None,
            },
        };

        let to_active = DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::IndexGenerationTransitioned {
                id: IndexGenerationId::new(10),
                from: IndexLifecycle::Shadow,
                to: IndexLifecycle::Active,
                replaced_active_id: None, // No previous active
            },
        };

        store.append(started).expect("append");
        store.append(to_evaluated).expect("append");
        store.append(to_shadow).expect("append");
        store.append(to_active).expect("append");
    }

    // Second session: scan with a fresh handle
    let store = SqliteStore::open(&db_path).expect("reopen");
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
    assert_eq!(scanned.len(), 4);

    // Replay into state
    let state = maestria_domain::replay_events(&scanned).expect("replay");

    // Assert active generation and fingerprint are preserved
    let active_gen = state
        .index_generations
        .get_active(&RepresentationName::new("dense_vector"))
        .expect("active generation should exist");

    assert_eq!(active_gen.id, IndexGenerationId::new(10));
    assert_eq!(active_gen.lifecycle, IndexLifecycle::Active);
    assert_eq!(active_gen.fingerprint, fingerprint);
}

#[test]
fn payload_rejects_missing_and_unknown_fields_for_generations() -> Result<(), PortError> {
    let missing_field = SqliteStore::in_memory()?;
    {
        let connection = missing_field.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'index_generation_started', NULL, ?1, 2)",
                params![
                    r#"{"event_kind":"index_generation_started","id":10,"name":"dense","corpus_snapshot":42}"# // missing fingerprint
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        missing_field.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));

    let unknown_field = SqliteStore::in_memory()?;
    {
        let connection = unknown_field.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'index_generation_transitioned', NULL, ?1, 2)",
                params![
                    r#"{"event_kind":"index_generation_transitioned","id":10,"from":"Building","to":"Evaluated","replaced_active_id":null,"unexpected":true}"#
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        unknown_field.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));

    let nested_unknown_field = SqliteStore::in_memory()?;
    {
        let connection = nested_unknown_field.lock()?;
        connection
            .execute(
                "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (1, 1, 'index_generation_started', NULL, ?1, 2)",
                params![
                    r#"{"event_kind":"index_generation_started","id":10,"name":"dense","corpus_snapshot":42,"fingerprint":{"provider":"p","model":"m","revision":"r","artifact_hash":"sha256:0000000000000000000000000000000000000000000000000000000000000000","dimensions":1,"quantization":"f32","query_template_hash":"q","document_template_hash":"d","preprocessing_version":"v","unexpected":true}}"#
                ],
            )
            .map_err(to_port_error)?;
    }
    assert!(matches!(
        nested_unknown_field.scan(EventFilter { artifact_id: None }),
        Err(PortError::Internal { .. })
    ));

    Ok(())
}
