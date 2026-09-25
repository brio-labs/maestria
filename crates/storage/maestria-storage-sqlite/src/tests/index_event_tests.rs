use crate::SqliteStore;
use maestria_domain::*;
use maestria_ports::*;

#[test]
fn pending_index_event_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(7),
            content_hash: maestria_test_support::content_hash(10)?,
        },
    };
    store.append(event.clone())?;
    let scanned = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(scanned, vec![event]);
    Ok(())
}

#[test]
fn full_text_indexed_event_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::FullTextIndexed {
            artifact_id: ArtifactId::new(7),
            chunk_id: ChunkId::new(42),
        },
    };
    store.append(event.clone())?;
    let scanned = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(scanned, vec![event]);
    Ok(())
}

#[test]
fn artifact_indexed_event_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(7),
        },
    };
    store.append(event.clone())?;
    let scanned = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(scanned, vec![event]);
    Ok(())
}

#[test]
fn parser_started_event_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(7),
            title: "test.md".to_string(),
            source_path: "/tmp/test.md".to_string(),
            content_hash: maestria_test_support::content_hash(13)?,
            blob_id: BlobId::new(42),
        },
    };
    store.append(event.clone())?;
    let scanned = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(scanned, vec![event]);
    Ok(())
}

#[test]
fn parser_started_event_filters_by_artifact() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let started = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: maestria_test_support::content_hash(10)?,
            blob_id: BlobId::new(10),
        },
    };
    let other = DomainEventEnvelope {
        id: EventId::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(2),
            title: "b.md".to_string(),
            source_path: "/tmp/b.md".to_string(),
            content_hash: maestria_test_support::content_hash(11)?,
            blob_id: BlobId::new(20),
        },
    };
    store.append(started.clone())?;
    store.append(other.clone())?;

    let for_artifact_1 = store.scan(EventFilter {
        artifact_id: Some(ArtifactId::new(1)),
    })?;
    assert_eq!(for_artifact_1, vec![started]);
    Ok(())
}

#[test]
fn parser_started_event_has_no_source_bytes_in_payload() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(7),
            title: "test.md".to_string(),
            source_path: "/tmp/test.md".to_string(),
            content_hash: maestria_test_support::content_hash(13)?,
            blob_id: BlobId::new(42),
        },
    };
    store.append(event)?;

    // Verify the raw stored JSON contains no source bytes
    let connection = store.lock()?;
    let payload_json: String = connection.query_row(
        "SELECT payload_json FROM domain_events WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    assert!(!payload_json.contains("source_bytes"));
    assert!(!payload_json.contains("source_blob"));
    Ok(())
}

#[test]
fn index_events_filter_by_artifact() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let pending = DomainEventEnvelope {
        id: EventId::new(1),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(1),
            content_hash: maestria_test_support::content_hash(10)?,
        },
    };
    let full_text = DomainEventEnvelope {
        id: EventId::new(2),
        event: DomainEvent::FullTextIndexed {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    };
    let indexed = DomainEventEnvelope {
        id: EventId::new(3),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(1),
        },
    };
    let other = DomainEventEnvelope {
        id: EventId::new(4),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(2),
        },
    };
    store.append(pending.clone())?;
    store.append(full_text.clone())?;
    store.append(indexed.clone())?;
    store.append(other.clone())?;

    let for_artifact_1 = store.scan(EventFilter {
        artifact_id: Some(ArtifactId::new(1)),
    })?;
    assert_eq!(for_artifact_1, vec![pending, full_text, indexed]);
    Ok(())
}

#[test]
fn source_event_scan_preserves_stale_and_restored_content_versions_without_audits()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let artifact_id = ArtifactId::new(7);
    let source_path = "/tmp/reapproved.md".to_string();
    let content_hash = maestria_test_support::content_hash(13)?;
    let version = content_hash.version_id()?;
    let started = DomainEvent::ParserStarted {
        artifact_id,
        title: "reapproved.md".to_string(),
        source_path: source_path.clone(),
        content_hash: content_hash.clone(),
        blob_id: BlobId::new(42),
    };
    store.append(DomainEventEnvelope {
        id: EventId::new(1),
        event: started.clone(),
    })?;
    store.append(DomainEventEnvelope {
        id: EventId::new(2),
        event: DomainEvent::SearchExecuted {
            query: "unrelated audit".to_string(),
            limit: 1,
            evidence_ids: Vec::new(),
            pack_metadata: None,
            at: LogicalTick::new(1),
        },
    })?;
    store.append(DomainEventEnvelope {
        id: EventId::new(3),
        event: DomainEvent::DocumentTreeCaptured {
            artifact_id,
            artifact_version_id: version,
            content_hash: content_hash.clone(),
            root_id: StructureNodeId::new(1),
            nodes: Vec::new(),
        },
    })?;
    store.append(DomainEventEnvelope {
        id: EventId::new(4),
        event: DomainEvent::SourceBecameStale {
            artifact_id,
            source_path: source_path.clone(),
            content_hash: content_hash.clone(),
        },
    })?;
    assert!(active_source_versions(&store.scan_searchable_source_events()?).is_empty());

    store.append(DomainEventEnvelope {
        id: EventId::new(5),
        event: started,
    })?;
    let source_events = store.scan_searchable_source_events()?;
    assert_eq!(
        source_events
            .iter()
            .map(|event| event.id.value())
            .collect::<Vec<_>>(),
        [1, 3, 4, 5]
    );
    assert_eq!(store.searchable_source_revision()?, 5);
    let active = active_source_versions(&source_events);
    assert_eq!(
        active,
        active_source_versions(&store.scan(EventFilter { artifact_id: None })?)
    );
    assert_eq!(
        active.get(std::path::Path::new(&source_path)),
        Some(&(artifact_id, version, content_hash))
    );
    Ok(())
}
