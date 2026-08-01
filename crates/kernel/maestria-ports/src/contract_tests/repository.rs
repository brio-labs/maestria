use super::*;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, ClaimId, ContentHash, DomainEvent,
    DomainEventEnvelope, EventId, Evidence, EvidenceId, EvidenceKind, LineRange, LogicalTick,
    SequenceNumber, SnapshotRef, ValidationReportId,
};

pub fn sample_artifact(id: u64) -> Artifact {
    Artifact {
        id: ArtifactId::new(id),
        title: format!("artifact-{id}"),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: maestria_domain::IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    }
}

pub fn assert_artifact_repository_round_trip(
    repository: &impl ArtifactRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = sample_artifact(1);

    repository.put(artifact.clone())?;

    assert_eq!(repository.get(artifact.id)?, Some(artifact));
    assert_eq!(repository.get(ArtifactId::new(99))?, None);
    Ok(())
}

pub fn assert_chunk_repository_round_trip(
    repository: &impl ChunkRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let first = Chunk {
        id: ChunkId::new(10),
        artifact_id: ArtifactId::new(1),
        order: 2,
        text: "second".to_string(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        representations: vec![],
    };
    let second = Chunk {
        id: ChunkId::new(11),
        artifact_id: ArtifactId::new(1),
        order: 1,
        text: "first".to_string(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        representations: vec![],
    };
    let unrelated = Chunk {
        id: ChunkId::new(12),
        artifact_id: ArtifactId::new(2),
        order: 0,
        text: "other".to_string(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        representations: vec![],
    };

    repository.put(first.clone())?;
    repository.put(second.clone())?;
    repository.put(unrelated)?;

    assert_eq!(repository.get(first.id)?, Some(first.clone()));
    assert_eq!(
        repository.find_artifact_id(first.id)?,
        Some(first.artifact_id)
    );
    assert_eq!(
        repository.list_for_artifact(ArtifactId::new(1))?,
        vec![second, first]
    );
    assert_eq!(repository.get(ChunkId::new(99))?, None);
    assert_eq!(repository.find_artifact_id(ChunkId::new(99))?, None);
    Ok(())
}

pub fn assert_card_repository_round_trip(
    repository: &impl CardRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let first = Card {
        id: CardId::new(20),
        artifact_id: ArtifactId::new(1),
        title: "bravo".to_string(),
        body: "body b".to_string(),
        claim_ids: [ClaimId::new(3), ClaimId::new(1)].into(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        security: maestria_domain::SecurityMetadata::default(),
    };
    let second = Card {
        id: CardId::new(21),
        artifact_id: ArtifactId::new(1),
        title: "alpha".to_string(),
        body: "body a".to_string(),
        claim_ids: Default::default(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        security: maestria_domain::SecurityMetadata::default(),
    };
    let unrelated = Card {
        id: CardId::new(22),
        artifact_id: ArtifactId::new(2),
        title: "other".to_string(),
        body: "body".to_string(),
        claim_ids: Default::default(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        security: maestria_domain::SecurityMetadata::default(),
    };

    repository.put(first.clone())?;
    repository.put(second.clone())?;
    repository.put(unrelated)?;

    assert_eq!(repository.get(first.id)?, Some(first.clone()));
    assert_eq!(
        repository.list_for_artifact(ArtifactId::new(1))?,
        vec![first, second]
    );
    assert_eq!(repository.get(CardId::new(99))?, None);
    Ok(())
}

pub fn assert_evidence_repository_round_trip(
    repository: &impl EvidenceRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = Evidence {
        id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(7)),
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_string(),
            range: LineRange::new(1, 4)?,
            snapshot: SnapshotRef::new(
                BlobId::new(40),
                ContentHash::new(maestria_domain::content_hash(b"source excerpt"))?,
            ),
        },
        excerpt: "source excerpt".to_string(),
        observed_at: LogicalTick::new(9),
        security: maestria_domain::SecurityMetadata::default(),
    };
    let validation = Evidence {
        id: EvidenceId::new(41),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(5),
        },
        excerpt: "validated".to_string(),
        observed_at: LogicalTick::new(10),
        security: maestria_domain::SecurityMetadata::default(),
    };
    let unrelated = Evidence {
        id: EvidenceId::new(42),
        artifact_id: ArtifactId::new(2),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(6),
        },
        excerpt: "other".to_string(),
        observed_at: LogicalTick::new(11),
        security: maestria_domain::SecurityMetadata::default(),
    };

    repository.put(file.clone())?;
    repository.put(validation.clone())?;
    repository.put(unrelated)?;

    assert_eq!(repository.get(file.id)?, Some(file.clone()));
    assert_eq!(
        repository.list_for_artifact(ArtifactId::new(1))?,
        vec![file, validation]
    );
    assert_eq!(repository.get(EvidenceId::new(99))?, None);
    Ok(())
}

pub fn assert_evidence_repository_replace_contract(
    repository: &impl EvidenceRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let original = Evidence {
        id: EvidenceId::new(50),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "original excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    let replacement = Evidence {
        id: EvidenceId::new(50),         // same id
        artifact_id: ArtifactId::new(2), // different artifact
        claim_id: Some(ClaimId::new(9)),
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "replacement excerpt".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };

    repository.put(original.clone())?;
    // put with different content must conflict
    let Err(err) = repository.put(replacement.clone()) else {
        return Err("expected error".into());
    };
    assert!(matches!(err, PortError::Conflict { .. }));
    // original still intact
    assert_eq!(repository.get(original.id)?, Some(original.clone()));
    // replace overwrites even with different content
    repository.replace(replacement.clone())?;
    assert_eq!(repository.get(replacement.id)?, Some(replacement.clone()));
    // replace of identical value is idempotent
    repository.replace(replacement.clone())?;
    assert_eq!(repository.get(replacement.id)?, Some(replacement.clone()));
    // replace on a fresh id acts as insert
    let fresh = Evidence {
        id: EvidenceId::new(51),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(3),
        },
        excerpt: "fresh".to_string(),
        observed_at: LogicalTick::new(3),
        security: maestria_domain::SecurityMetadata::default(),
    };
    repository.replace(fresh.clone())?;
    assert_eq!(repository.get(fresh.id)?, Some(fresh));
    Ok(())
}

pub fn assert_event_log_round_trip(log: &impl EventLog) -> Result<(), Box<dyn std::error::Error>> {
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "notes".to_string(),
            security: maestria_domain::SecurityMetadata::default(),
        },
    };
    let evidence = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::EvidenceRecorded {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range: LineRange::new(1, 4)?,
                snapshot: SnapshotRef::new(
                    BlobId::new(40),
                    ContentHash::new(maestria_domain::content_hash(b"excerpt"))?,
                ),
            },
            excerpt: "excerpt".to_string(),
            observed_at: LogicalTick::new(0),
            security: maestria_domain::SecurityMetadata::default(),
        },
    };
    let search = DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::SearchCompleted {
            artifact_id: ArtifactId::new(1),
            cards_added: 2,
        },
    };
    let unrelated = DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(2),
            title: "other".to_string(),
            security: maestria_domain::SecurityMetadata::default(),
        },
    };

    log.append(event.clone())?;
    log.append(evidence.clone())?;
    log.append(search.clone())?;
    log.append(unrelated)?;

    let out_of_order = DomainEventEnvelope {
        id: EventId::new(6), // next is 5
        sequence: SequenceNumber::new(6),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(0),
        },
    };
    let Err(err) = log.append(out_of_order) else {
        return Err("expected error".into());
    };
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "out of order must return Conflict"
    );

    let id_mismatch = DomainEventEnvelope {
        id: EventId::new(99),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(0),
        },
    };
    let Err(err_id) = log.append(id_mismatch) else {
        return Err("expected error".into());
    };
    assert!(
        matches!(err_id, PortError::Conflict { .. }),
        "id mismatch must return Conflict"
    );

    let all = log.scan(EventFilter { artifact_id: None })?;
    assert_eq!(all.len(), 4);

    let filtered = log.scan(EventFilter {
        artifact_id: Some(ArtifactId::new(1)),
    })?;
    assert_eq!(filtered, vec![event, evidence, search]);
    Ok(())
}

pub fn assert_blob_store_round_trip(
    store: &impl BlobStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let first = store.put(vec![1, 2, 3])?;
    let first_duplicate = store.put(vec![1, 2, 3])?;
    let second = store.put(vec![4, 5])?;

    assert_eq!(first, first_duplicate);
    assert_ne!(first, second);
    assert_eq!(store.get(first)?, vec![1, 2, 3]);
    assert_eq!(store.get(second)?, vec![4, 5]);
    assert!(matches!(
        store.get(BlobId::new(99)),
        Err(PortError::NotFound)
    ));
    Ok(())
}
