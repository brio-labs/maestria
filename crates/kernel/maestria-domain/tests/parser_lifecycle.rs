use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

fn count_chunk_events(output: &KernelOutput) -> usize {
    output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ChunkRegistered { .. }))
        .count()
}

fn count_card_events(output: &KernelOutput) -> usize {
    output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::CardCreated { .. }))
        .count()
}

fn count_parsed_events(output: &KernelOutput) -> usize {
    output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count()
}

// ── Parser lifecycle: completed, started, idempotency ─────────────

#[test]
fn parser_completed_duplicate_is_idempotent() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
    }))?;

    let parser_result = ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
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
        cards: vec![CreateCardInput {
            node_id: StructureNodeId::new(10),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            card_id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Parsed summary".to_string(),
        }],
    };

    // First parse: creates chunk, card, ArtifactParsed
    let output1 = state.apply_input(DomainInput::ParserCompleted(parser_result.clone()))?;
    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    assert!(state.cards.contains_key(&CardId::new(20)));
    assert_eq!(
        count_chunk_events(&output1),
        1,
        "first parse registers chunk"
    );
    assert_eq!(count_card_events(&output1), 1, "first parse creates card");
    assert_eq!(
        count_parsed_events(&output1),
        1,
        "first parse emits ArtifactParsed"
    );

    // Second parse with identical data: no duplicate events
    let output2 = state.apply_input(DomainInput::ParserCompleted(parser_result))?;
    assert_eq!(
        count_chunk_events(&output2),
        0,
        "duplicate parse emits no chunk events"
    );
    assert_eq!(
        count_card_events(&output2),
        0,
        "duplicate parse emits no card events"
    );
    assert_eq!(
        count_parsed_events(&output2),
        0,
        "duplicate parse with no new chunks/cards emits no ArtifactParsed"
    );
    assert_eq!(
        output2.events.len(),
        0,
        "duplicate parse emits no tree, chunk, or card events"
    );
    Ok(())
}

#[test]
fn parser_completed_rejects_mismatched_chunk() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
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

    // Second parse with same chunk_id but different text
    let err = state
        .apply_input(DomainInput::ParserCompleted(ParserResult {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            artifact_version_id: ArtifactVersionId::new(1),
            content_hash: fixtures::test_content_hash(),
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
                text: "different text".to_string(),
            }],
            cards: Vec::new(),
        }))
        .expect_err("mismatched chunk must error");

    assert!(matches!(err, DomainError::DuplicateId { kind, id: 10 } if kind == "chunk"));
    Ok(())
}

#[test]
fn parser_completed_rejects_mismatched_card() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![
            fixtures::tree_root_node(StructureNodeId::new(0)),
            maestria_domain::StructureNode {
                id: StructureNodeId::new(1),
                parent_id: Some(StructureNodeId::new(0)),
                sibling_id: None,
                node_type: maestria_domain::StructureNodeType::Paragraph,
                source_range: maestria_domain::ContentRange { start: 0, end: 0 },
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            },
        ],
        chunks: Vec::new(),
        cards: vec![CreateCardInput {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            card_id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Parsed summary".to_string(),
        }],
    }))?;

    // Second parse with same card_id but different body
    let err = state
        .apply_input(DomainInput::ParserCompleted(ParserResult {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            artifact_version_id: ArtifactVersionId::new(1),
            content_hash: fixtures::test_content_hash(),
            tree_root_id: Some(StructureNodeId::new(0)),
            tree_nodes: vec![
                fixtures::tree_root_node(StructureNodeId::new(0)),
                maestria_domain::StructureNode {
                    id: StructureNodeId::new(1),
                    parent_id: Some(StructureNodeId::new(0)),
                    sibling_id: None,
                    node_type: maestria_domain::StructureNodeType::Paragraph,
                    source_range: maestria_domain::ContentRange { start: 0, end: 0 },
                    page: None,
                    section_path: vec![],
                    parser_generation: "test".to_string(),
                    schema_generation: "1".to_string(),
                    language: None,
                },
            ],
            chunks: Vec::new(),
            cards: vec![CreateCardInput {
                node_id: maestria_domain::StructureNodeId::new(1),
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                card_id: CardId::new(20),
                artifact_id: ArtifactId::new(1),
                title: "Summary".to_string(),
                body: "different body".to_string(),
            }],
        }))
        .expect_err("mismatched card must error");

    assert!(matches!(err, DomainError::DuplicateId { kind, id: 20 } if kind == "card"));
    Ok(())
}

#[test]
fn parser_started_stores_metadata_and_emits_persist_event() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;

    // Pending-parser metadata is stored in-memory.
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));
    let pending = &state.pending_parsers[&ArtifactId::new(1)];
    assert_eq!(pending.title, "Notes");
    assert_eq!(pending.source_path, "/tmp/notes.md");
    assert_eq!(pending.content_hash, "sha256:abc");
    assert_eq!(pending.blob_id, BlobId::new(42));

    // Exactly one PersistEvent carrying the ParserStarted event.
    assert_eq!(output.events.len(), 1);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::ParserStarted {
            artifact_id: ArtifactId(1),
            title,
            source_path,
            content_hash,
            blob_id: BlobId(42),
        } if title == "Notes" && source_path == "/tmp/notes.md" && content_hash == "sha256:abc"
    ));
    assert_eq!(output.effects.len(), 1);
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::PersistEvent { .. }
    ));

    // No artifact created yet — ParserStarted is pure metadata.
    assert!(state.artifacts.is_empty());
    Ok(())
}

#[test]
fn resume_parser_emits_parse_artifact_with_source_blob() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Set up: pending_parsers exists from replay
    state.pending_parsers.insert(
        ArtifactId::new(1),
        ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    );

    let output = state.apply_input(DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;

    // No events — ResumeParser re-drives only, no new persisted metadata.
    assert_eq!(output.events.len(), 0);
    assert_eq!(output.effects.len(), 1);
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::ParseArtifact(req)
            if req.artifact_id == ArtifactId::new(1)
            && req.source_blob == Some(BlobId::new(42))
            && req.source_bytes.is_empty()
    ));
    Ok(())
}

#[test]
fn resume_parser_without_pending_entry_is_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // pending_parsers is empty — no entry to resume
    let err = state
        .apply_input(DomainInput::ResumeParser(ParserStarted {
            artifact_id: ArtifactId::new(99),
            title: "Ghost".to_string(),
            source_path: String::new(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(1),
        }))
        .expect_err("resume without pending entry must error");
    assert!(matches!(
        err,
        DomainError::MissingArtifact { id } if id == ArtifactId::new(99)
    ));
    Ok(())
}

#[test]
fn parser_completed_removes_pending_parser() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Set up: preflight detection + parser started
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // Parser completes — pending_parsers survives until ArtifactIndexed.
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    }))?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers retained after ParserCompleted; cleared only at ArtifactIndexed"
    );
    assert!(
        !state.pending_artifacts.contains_key(&ArtifactId::new(1)),
        "pending_artifacts consumed on ParserCompleted"
    );
    // First zero-output parse emits ArtifactParsed.
    assert!(output.events.iter().any(|e| matches!(
        e.event,
        DomainEvent::ArtifactParsed {
            status: _,
            chunks_added: 0,
            ..
        }
    )));
    Ok(())
}

#[test]
fn replay_reconstructs_pending_parsers() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    })?;

    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));
    let pending = &state.pending_parsers[&ArtifactId::new(1)];
    assert_eq!(pending.title, "Notes");
    assert_eq!(pending.blob_id, BlobId::new(42));
    Ok(())
}

#[test]
fn replay_artifact_parsed_cleans_up_pending_parsers() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Set up: artifact registered (from first-time commit) + parser started
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    })?;
    // Replay the ArtifactParsed event (emitted on ParserCompleted success).
    // pending_parsers is NOT removed here — only ArtifactIndexed clears it.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ArtifactParsed {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            chunks_added: 0,
        },
    })?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactParsed replay retains pending parsers; only ArtifactIndexed clears them"
    );
    Ok(())
}

#[test]
fn full_ingestion_flow_with_parser_started() -> Result<(), DomainError> {
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
        content_hash: fixtures::test_content_hash(),
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
-> Result<(), DomainError> {
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
        content_hash: fixtures::test_content_hash(),
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
fn parser_completed_resume_pending_same_hash_is_idempotent() -> Result<(), DomainError> {
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
        content_hash: fixtures::test_content_hash(),
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
        .any(|e| matches!(e.event, DomainEvent::ArtifactParsed { status: _, .. }));
    assert!(has_parsed, "ArtifactParsed still emitted");

    // Assert: pending_parsers retained (cleared only at ArtifactIndexed).
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    Ok(())
}

// ── Parser metadata retention and duplicate suppression ──────────

#[test]
fn parser_completed_first_zero_output_emits_artifact_parsed() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
    }))?;

    // First parse with zero chunks/cards must still emit ArtifactParsed.
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    }))?;

    let parsed_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { status: _, .. }))
        .count();
    assert_eq!(
        parsed_count, 1,
        "first zero-output parse emits ArtifactParsed"
    );
    Ok(())
}

#[test]
fn parser_completed_duplicate_zero_output_suppresses_artifact_parsed() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
    }))?;

    let empty_result = ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    };

    // First parse.
    let output1 = state.apply_input(DomainInput::ParserCompleted(empty_result.clone()))?;
    assert!(
        output1
            .events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::ArtifactParsed { status: _, .. }))
    );

    // Second parse with same zero data — must suppress duplicate ArtifactParsed.
    let output2 = state.apply_input(DomainInput::ParserCompleted(empty_result))?;
    let parsed2 = output2
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { status: _, .. }))
        .count();
    assert_eq!(
        parsed2, 0,
        "duplicate zero-output parse suppresses ArtifactParsed"
    );
    Ok(())
}
