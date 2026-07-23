use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

// ── Crash-recovery resume flows ───────────────────────────────────

#[test]
fn full_ingestion_flow_with_parser_started() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();

    // 1. Detection
    let output = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    assert_eq!(output.events.len(), 0, "detection emits no events");
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::ParseArtifact(req) if req.source_blob.is_none()
    ));

    // 2. ParserStarted (runtime stores blob, then reports metadata)
    let output = state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::ParserStarted { .. }
    ));

    // 3. ParserCompleted — pending_parsers retained until ArtifactIndexed.
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "chunk".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers retained after ParserCompleted; cleared only at ArtifactIndexed"
    );
    assert!(state.artifacts.contains_key(&ArtifactId::new(1)));
    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    assert!(output.events.iter().any(|e| matches!(
        e.event,
        DomainEvent::ArtifactParsed {
            status: _,
            artifact_id: ArtifactId(1),
            ..
        }
    )));
    Ok(())
}

#[test]
fn parser_completed_resume_with_artifact_registered_restores_pending_index()
-> Result<(), Box<dyn std::error::Error>> {
    // Crash scenario: ArtifactRegistered event appended, ParserStarted event
    // appended, but PendingIndex event NOT appended before crash. On replay,
    // the artifact exists (from ArtifactRegistered) with Unindexed status,
    // and pending_parsers has the ParserStarted metadata. ParserCompleted
    // must restore PendingIndex and pending full-text tracking.
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(1),
                title: "Notes".to_string(),
                security: SecurityMetadata::default(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: ArtifactId::new(1),
                title: "Notes".to_string(),
                source_path: "/tmp/notes.md".to_string(),
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(42),
            },
        },
    ];
    let mut state = replay_events(&events)?;

    // Pre-conditions: artifact exists (Unindexed, no hash), pending_parsers set.
    assert!(state.artifacts.contains_key(&ArtifactId::new(1)));
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Unindexed,
    );
    assert!(state.artifacts[&ArtifactId::new(1)].content_hash.is_none());
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // Act: ParserCompleted on resume.
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "first chunk".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // Assert: PendingIndex event was emitted.
    let pending_idx = output
        .events
        .iter()
        .find(|e| matches!(e.event, DomainEvent::PendingIndex { .. }));
    assert!(
        pending_idx.is_some(),
        "PendingIndex must be emitted on resume after crash"
    );
    if let Some(envelope) = pending_idx
        && let DomainEvent::PendingIndex {
            artifact_id,
            content_hash,
        } = &envelope.event
    {
        assert_eq!(*artifact_id, ArtifactId::new(1));
        assert_eq!(content_hash, "sha256:abc");
    }

    // Assert: pending_parsers retained (cleared only at ArtifactIndexed).
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // Assert: chunk is tracked in pending_full_text.
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));

    // Assert: ArtifactParsed event emitted.
    let has_parsed = output.events.iter().any(|e| {
        matches!(
            e.event,
            DomainEvent::ArtifactParsed {
                status: _,
                chunks_added: 1,
                ..
            }
        )
    });
    assert!(
        has_parsed,
        "ArtifactParsed must be emitted with chunk count"
    );

    Ok(())
}

#[test]
fn parser_completed_resume_pending_same_hash_is_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    // On resume with ArtifactRegistered + PendingIndex already replayed
    // (both events durable), a ParserCompleted retry with the same hash
    // must not emit a duplicate PendingIndex.
    let events = vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(1),
                title: "Notes".to_string(),
                security: SecurityMetadata::default(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: ArtifactId::new(1),
                title: "Notes".to_string(),
                source_path: "/tmp/notes.md".to_string(),
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: ArtifactId::new(1),
                content_hash: "sha256:abc".to_string(),
            },
        },
    ];
    let mut state = replay_events(&events)?;

    // Pre-condition: artifact is already Pending with correct hash.
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending,
    );
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].content_hash.as_deref(),
        Some("sha256:abc"),
    );
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // Act: ParserCompleted retry.
    let result = ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "chunk".to_string(),
        }],
        cards: Vec::new(),
    };
    let output = state.apply_input(DomainInput::ParserCompleted(result.clone()))?;

    // Assert: no PendingIndex in output (already Pending, same hash).
    let pending_events: Vec<_> = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::PendingIndex { .. }))
        .collect();
    assert!(
        pending_events.is_empty(),
        "must not emit duplicate PendingIndex when already Pending with same hash"
    );

    // Assert: chunk registered, ArtifactParsed emitted.
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));
    let has_parsed = output
        .events
        .iter()
        .any(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }));
    assert!(has_parsed, "ArtifactParsed still emitted");

    // Assert: pending_parsers retained (cleared only at ArtifactIndexed).
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    Ok(())
}

// ── Parser metadata retention and duplicate suppression ──────────
