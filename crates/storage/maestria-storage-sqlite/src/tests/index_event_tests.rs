use crate::SqliteStore;
use maestria_domain::*;
use maestria_ports::*;

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
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
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
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
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
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn parser_started_event_round_trips() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(7),
            title: "test.md".to_string(),
            source_path: "/tmp/test.md".to_string(),
            content_hash: "sha256:def456".to_string(),
            blob_id: BlobId::new(42),
        },
    };
    store.append(event.clone()).expect("append");
    let scanned = store.scan(EventFilter { artifact_id: None }).expect("scan");
    assert_eq!(scanned, vec![event]);
}

#[test]
fn parser_started_event_filters_by_artifact() {
    let store = SqliteStore::in_memory().expect("test setup");
    let started = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: "sha256:aaa".to_string(),
            blob_id: BlobId::new(10),
        },
    };
    let other = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(2),
            title: "b.md".to_string(),
            source_path: "/tmp/b.md".to_string(),
            content_hash: "sha256:bbb".to_string(),
            blob_id: BlobId::new(20),
        },
    };
    store.append(started.clone()).expect("append");
    store.append(other.clone()).expect("append");

    let for_artifact_1 = store
        .scan(EventFilter {
            artifact_id: Some(ArtifactId::new(1)),
        })
        .expect("filtered scan");
    assert_eq!(for_artifact_1, vec![started]);
}

#[test]
fn parser_started_event_has_no_source_bytes_in_payload() {
    let store = SqliteStore::in_memory().expect("test setup");
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(7),
            title: "test.md".to_string(),
            source_path: "/tmp/test.md".to_string(),
            content_hash: "sha256:def456".to_string(),
            blob_id: BlobId::new(42),
        },
    };
    store.append(event).expect("append");

    // Verify the raw stored JSON contains no source bytes
    let connection = store.lock().expect("lock");
    let payload_json: String = connection
        .query_row(
            "SELECT payload_json FROM domain_events WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("query");
    assert!(!payload_json.contains("source_bytes"));
    assert!(!payload_json.contains("source_blob"));
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
