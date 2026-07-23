use maestria_domain::*;

/// Event log that replays a malformed deterministic evidence record followed
/// by a valid replacement at the same ID.
pub fn malformed_to_valid_replacement_events(
    art_id: ArtifactId,
    chunk_id: ChunkId,
    ev_id: EvidenceId,
) -> Vec<DomainEventEnvelope> {
    vec![
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
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: art_id,
                content_hash: "sha256:abc".to_string(),
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
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
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
        // Valid replacement (FileSpan with snapshot and correct hash).
        DomainEventEnvelope {
            id: EventId::new(7),
            sequence: SequenceNumber::new(7),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/tmp/test.md".to_string(),
                    range: ContentRange { start: 0, end: 1 },
                    content_hash: "sha256:abc".to_string(),
                    snapshot: Some(BlobId::new(42)),
                },
                excerpt: "hello".to_string(),
                observed_at: LogicalTick::new(2),
                security: SecurityMetadata::default(),
            },
        },
    ]
}

/// Event log with two *different* valid deterministic evidence records at the
/// same ID — replay must reject the second as a duplicate.
pub fn valid_duplicate_evidence_events() -> Vec<DomainEventEnvelope> {
    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let ev_id = evidence_id_for(art_id, 0);
    vec![
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
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: art_id,
                content_hash: "sha256:abc".to_string(),
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
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
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
                    range: ContentRange { start: 0, end: 1 },
                    content_hash: "sha256:abc".to_string(),
                    snapshot: Some(BlobId::new(42)),
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
                    range: ContentRange { start: 1, end: 2 },
                    content_hash: "sha256:abc".to_string(),
                    snapshot: Some(BlobId::new(42)),
                },
                excerpt: "different".to_string(),
                observed_at: LogicalTick::new(2),
                security: SecurityMetadata::default(),
            },
        },
    ]
}
