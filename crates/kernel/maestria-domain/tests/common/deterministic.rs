use maestria_domain::*;
#[path = "content_hash.rs"]
mod fixtures;

/// Event log that replays a malformed deterministic evidence record.
pub fn malformed_deterministic_evidence_events(
    art_id: ArtifactId,
    chunk_id: ChunkId,
    ev_id: EvidenceId,
) -> Result<Vec<DomainEventEnvelope>, Box<dyn std::error::Error>> {
    let content_hash = fixtures::test_content_hash()?;
    Ok(vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: art_id,
                title: "Test".to_string(),
                security: SecurityMetadata::default(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: art_id,
                title: "Test".to_string(),
                source_path: "/tmp/test.md".to_string(),
                content_hash: content_hash.clone(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: art_id,
                content_hash: content_hash.clone(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::ArtifactParsed {
                status: maestria_domain::ParseStatus::Parsed,
                artifact_id: art_id,
                chunks_added: 1,
            },
        },
        DomainEventEnvelope {
            id: EventId::new(5),
            sequence: SequenceNumber::new(5),
            event: DomainEvent::ChunkRegistered {
                node_id: StructureNodeId::new(1),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                chunk_id,
                artifact_id: art_id,
                order: 0,
                text: "hello".to_string(),
            },
        },
        // Malformed evidence record (CommandOutput, not FileSpan).
        DomainEventEnvelope {
            id: EventId::new(6),
            sequence: SequenceNumber::new(6),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::CommandOutput {
                    harness_run: HarnessRunId::new(1),
                    stream: OutputStream::Stdout,
                    blob: BlobId::new(99),
                },
                excerpt: "old".to_string(),
                observed_at: LogicalTick::new(1),
                security: SecurityMetadata::default(),
            },
        },
    ])
}

/// Event log with two *different* valid deterministic evidence records at the
/// same ID — replay must reject the second as a duplicate.
pub fn valid_duplicate_evidence_events()
-> Result<Vec<DomainEventEnvelope>, Box<dyn std::error::Error>> {
    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let ev_id = evidence_id_for(art_id, 0);
    let content_hash = fixtures::test_content_hash()?;
    Ok(vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: art_id,
                title: "Test".to_string(),
                security: SecurityMetadata::default(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: art_id,
                title: "Test".to_string(),
                source_path: "/tmp/test.md".to_string(),
                content_hash: content_hash.clone(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: art_id,
                content_hash: content_hash.clone(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::ArtifactParsed {
                status: maestria_domain::ParseStatus::Parsed,
                artifact_id: art_id,
                chunks_added: 1,
            },
        },
        DomainEventEnvelope {
            id: EventId::new(5),
            sequence: SequenceNumber::new(5),
            event: DomainEvent::ChunkRegistered {
                node_id: StructureNodeId::new(1),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                chunk_id,
                artifact_id: art_id,
                order: 0,
                text: "hello".to_string(),
            },
        },
        // Valid evidence.
        DomainEventEnvelope {
            id: EventId::new(6),
            sequence: SequenceNumber::new(6),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/tmp/test.md".to_string(),
                    range: LineRange::new(1, 1)?,
                    snapshot: SnapshotRef::new(BlobId::new(42), content_hash.clone()),
                },
                excerpt: "hello".to_string(),
                observed_at: LogicalTick::new(1),
                security: SecurityMetadata::default(),
            },
        },
        // Different valid evidence at same ID — must error.
        DomainEventEnvelope {
            id: EventId::new(7),
            sequence: SequenceNumber::new(7),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/tmp/test.md".to_string(),
                    range: LineRange::new(2, 2)?,
                    snapshot: SnapshotRef::new(BlobId::new(42), content_hash),
                },
                excerpt: "different".to_string(),
                observed_at: LogicalTick::new(2),
                security: SecurityMetadata::default(),
            },
        },
    ])
}
