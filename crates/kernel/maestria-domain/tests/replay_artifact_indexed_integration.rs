use maestria_domain::*;

#[path = "common/content_hash.rs"]
mod fixtures;

fn new_envelope(id: u64, event: DomainEvent) -> DomainEventEnvelope {
    DomainEventEnvelope {
        id: EventId::new(id),
        sequence: SequenceNumber::new(id),
        event,
    }
}

struct ReplayArtifactSetup<'a> {
    art_id: ArtifactId,
    title: &'a str,
    source_path: &'a str,
    content_hash: &'a str,
    blob_id: BlobId,
    chunk_id: ChunkId,
    chunk_text: &'a str,
}

/// Applies the initial 4-event setup: ArtifactRegistered(1), ParserStarted(2),
/// PendingIndex(3), ChunkRegistered(4). Asserts pending_parsers after ParserStarted.
fn replay_setup_artifact(
    state: &mut KernelState,
    setup: ReplayArtifactSetup<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ReplayArtifactSetup {
        art_id,
        title,
        source_path,
        content_hash,
        blob_id,
        chunk_id,
        chunk_text,
    } = setup;
    state.apply_event(new_envelope(
        1,
        DomainEvent::ArtifactRegistered {
            artifact_id: art_id,
            title: title.to_string(),
            security: SecurityMetadata::default(),
        },
    ))?;
    state.apply_event(new_envelope(
        2,
        DomainEvent::ParserStarted {
            artifact_id: art_id,
            title: title.to_string(),
            source_path: source_path.to_string(),
            content_hash: content_hash.to_string(),
            blob_id,
        },
    ))?;
    assert!(state.pending_parsers.contains_key(&art_id));
    state.apply_event(new_envelope(
        3,
        DomainEvent::PendingIndex {
            artifact_id: art_id,
            content_hash: content_hash.to_string(),
        },
    ))?;
    state.apply_event(new_envelope(
        4,
        DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::text_span(1, 1)?,
            representations: vec![],
            chunk_id,
            artifact_id: art_id,
            order: 0,
            text: chunk_text.to_string(),
        },
    ))?;
    Ok(())
}

#[test]
fn replay_artifact_indexed_clears_pending_parsers() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Full chain: ArtifactRegistered → ParserStarted → PendingIndex
    // → ChunkRegistered → EvidenceRecorded → FullTextIndexed
    // → ArtifactParsed → ArtifactIndexed.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            security: SecurityMetadata::default(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: fixtures::test_content_hash()?.as_str().to_string(),
            blob_id: BlobId::new(42),
        },
    })?;
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // PendingIndex is required to set content_hash on the artifact
    // so the evidence-completeness gate can match hashes.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(1),
            content_hash: fixtures::test_content_hash()?.as_str().to_string(),
        },
    })?;

    // Register a chunk so the FullTextIndexed → ArtifactIndexed chain works.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::text_span(1, 1)?,
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "hello".to_string(),
        },
    })?;

    // Record source-backed FileSpan evidence with snapshot and
    // matching content_hash so the evidence-completeness gate passes.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::EvidenceRecorded {
            evidence_id: evidence_id_for(ArtifactId::new(1), 0),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/notes.md".to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(BlobId::new(42), fixtures::test_content_hash()?),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
            security: SecurityMetadata::default(),
        },
    })?;

    // FullTextIndexed must be replayed so pending_full_text is empty
    // when ArtifactIndexed arrives.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(6),
        sequence: SequenceNumber::new(6),
        event: DomainEvent::FullTextIndexed {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    })?;

    // ArtifactParsed must NOT clear pending_parsers.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(7),
        sequence: SequenceNumber::new(7),
        event: DomainEvent::ArtifactParsed {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            chunks_added: 1,
        },
    })?;
    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactParsed must not clear pending_parsers"
    );

    // ArtifactIndexed (terminal) MUST clear pending_parsers when
    // evidence is complete.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(8),
        sequence: SequenceNumber::new(8),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(1),
        },
    })?;
    assert!(
        !state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactIndexed must clear pending_parsers on replay"
    );
    Ok(())
}

#[test]
fn replay_artifact_indexed_rejects_incomplete_evidence() -> Result<(), Box<dyn std::error::Error>> {
    // Incomplete ArtifactIndexed replay is rejected atomically before the
    // event can enter the append-only log or alter recovery state.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: art_id,
            title: "Notes".to_string(),
            security: SecurityMetadata::default(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: art_id,
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: fixtures::test_content_hash()?.as_str().to_string(),
            blob_id: BlobId::new(42),
        },
    })?;
    assert!(state.pending_parsers.contains_key(&art_id));

    // No PendingIndex, no EvidenceRecorded → evidence_complete_for is false.
    // Set content_hash via PendingIndex so the artifact exists but lacks evidence.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::PendingIndex {
            artifact_id: art_id,
            content_hash: fixtures::test_content_hash()?.as_str().to_string(),
        },
    })?;

    // ChunkRegistered so pending_chunks check passes.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::text_span(1, 1)?,
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: art_id,
            order: 0,
            text: "hello".to_string(),
        },
    })?;

    // FullTextIndexed so pending_full_text is clear.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::FullTextIndexed {
            artifact_id: art_id,
            chunk_id: ChunkId::new(10),
        },
    })?;

    let before = state.clone();
    let err = match state.apply_event(DomainEventEnvelope {
        id: EventId::new(6),
        sequence: SequenceNumber::new(6),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: art_id,
        },
    }) {
        Ok(()) => return Err("incomplete ArtifactIndexed unexpectedly replayed".into()),
        Err(error) => error,
    };
    assert!(matches!(
        err,
        DomainError::MissingEvidence { id } if id == evidence_id_for(art_id, 0)
    ));
    assert_eq!(state, before, "invalid replay must be atomic");
    assert_eq!(state.event_log.len(), 5);
    Ok(())
}

#[test]
fn replay_artifact_indexed_rejects_invalid_evidence() -> Result<(), Box<dyn std::error::Error>> {
    // Invalid ArtifactIndexed replay must fail before mutating evidence,
    // artifact state, or the append-only event log.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    let det_ev_id = evidence_id_for(art_id, 0);
    let content_hash = fixtures::test_content_hash()?;
    let wrong_content_hash = ContentHash::new("sha256:".to_owned() + &"f".repeat(64))?;

    replay_setup_artifact(
        &mut state,
        ReplayArtifactSetup {
            art_id,
            title: "Notes",
            source_path: "/tmp/notes.md",
            content_hash: content_hash.as_str(),
            blob_id: BlobId::new(42),
            chunk_id: ChunkId::new(10),
            chunk_text: "hello",
        },
    )?;

    let before = state.clone();
    let error = match state.apply_event(new_envelope(
        5,
        DomainEvent::EvidenceRecorded {
            evidence_id: det_ev_id,
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/notes.md".to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(BlobId::new(42), wrong_content_hash),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
            security: SecurityMetadata::default(),
        },
    )) {
        Ok(()) => return Err("mismatched evidence snapshot unexpectedly replayed".into()),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DomainError::MalformedDeterministicEvidence { evidence_id, .. }
            if evidence_id == det_ev_id
    ));
    assert_eq!(state, before, "invalid evidence replay must be atomic");
    assert!(!state.evidences.contains_key(&det_ev_id));
    Ok(())
}
