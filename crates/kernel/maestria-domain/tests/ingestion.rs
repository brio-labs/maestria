use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;
fn require_error<T, E>(
    result: Result<T, E>,
    message: &str,
) -> Result<E, Box<dyn std::error::Error>> {
    match result {
        Ok(_) => Err(std::io::Error::other(message).into()),
        Err(error) => Ok(error),
    }
}
fn parser_result_two_chunks() -> Result<ParserResult, Box<dyn std::error::Error>> {
    Ok(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                order: 0,
                text: "a".to_string(),
            },
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(11),
                order: 1,
                text: "b".to_string(),
            },
        ],
        cards: Vec::new(),
    })
}
fn record_file_evidence(
    state: &mut KernelState,
    order: u32,
    start: usize,
    end: usize,
    excerpt: &str,
) -> Result<(), DomainError> {
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: evidence_id_for(ArtifactId::new(1), order),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/notes.md".to_string(),
            range: ContentRange { start, end },
            content_hash: "sha256:abc".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: excerpt.to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;
    Ok(())
}
fn index_chunk(state: &mut KernelState, chunk_id: u64) -> Result<(), DomainError> {
    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(chunk_id),
        },
    ))?;
    Ok(())
}
fn replay_assert_indexed_parity(state: &KernelState) -> Result<(), DomainError> {
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state.artifacts, replayed.artifacts, "artifacts match");
    assert_eq!(state.chunks, replayed.chunks, "chunks match");
    assert_eq!(state.event_log, replayed.event_log, "event log matches");
    assert_eq!(
        state.pending_full_text, replayed.pending_full_text,
        "pending full text matches"
    );
    assert!(
        replayed.document_trees.contains_key(&ArtifactId::new(1)),
        "replay populates document_trees"
    );
    assert!(
        replayed.artifact_versions.contains_key(&ArtifactId::new(1)),
        "replay populates artifact_versions"
    );
    assert_eq!(
        replayed.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert!(replayed.pending_full_text.is_empty());
    Ok(())
}
// ── Ingestion pipeline: detection, parsing, full-text indexing ────
#[test]
fn preflight_artifact_detected_stores_pending_only() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;

    // ArtifactDetected stores metadata only in the in-memory pending map — no
    // persisted artifact or events.
    assert!(
        state.artifacts.is_empty(),
        "no artifact persisted at detection"
    );
    assert!(state.pending_artifacts.contains_key(&ArtifactId::new(1)));
    let pending = &state.pending_artifacts[&ArtifactId::new(1)];
    assert_eq!(pending.title, "Notes");
    assert_eq!(pending.content_hash, "sha256:abc");

    // No domain events emitted; only the ParseArtifact effect.
    assert_eq!(output.events.len(), 0, "no persisted events at detection");
    assert_eq!(output.effects.len(), 1);
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::ParseArtifact(req) if req.artifact_id == ArtifactId::new(1)
    ));
    Ok(())
}

#[test]
fn preflight_duplicate_is_noop_when_indexed() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Set up artifact as fully indexed via replay events
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            security: maestria_domain::SecurityMetadata::default(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(1),
            content_hash: "sha256:abc".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(1),
        },
    })?;

    // Re-detection with same hash while Indexed is a no-op
    let output = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;

    assert_eq!(
        output.events.len(),
        0,
        "no events for unchanged indexed artifact"
    );
    assert_eq!(
        output.effects.len(),
        0,
        "no effects for unchanged indexed artifact"
    );
    Ok(())
}

#[test]
fn detection_without_parser_leaves_pending_only() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    // Detection is a pure preflight — artifact lives in pending_artifacts only.
    assert!(!state.artifacts.contains_key(&ArtifactId::new(1)));
    assert!(state.pending_artifacts.contains_key(&ArtifactId::new(1)));
    assert!(state.chunks.is_empty(), "no chunks before parsing");
    assert!(state.cards.is_empty(), "no cards before parsing");
    Ok(())
}

#[test]
fn parser_without_prior_detection_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let err = require_error(
        state.apply_input(DomainInput::ParserCompleted(ParserResult {
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
                text: "lonely chunk".to_string(),
            }],
            cards: Vec::new(),
        })),
        "parser without detection must error",
    )?;
    assert!(matches!(err, DomainError::MissingArtifact { id } if id == ArtifactId::new(1)));
    Ok(())
}

fn two_chunk_parser_result(
    first: &str,
    second: &str,
    card_body: &str,
) -> Result<ParserResult, Box<dyn std::error::Error>> {
    Ok(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![
            fixtures::tree_root_node(StructureNodeId::new(10)),
            maestria_domain::StructureNode {
                id: StructureNodeId::new(11),
                parent_id: Some(StructureNodeId::new(10)),
                sibling_id: None,
                node_type: maestria_domain::StructureNodeType::Paragraph,
                source_range: ContentRange { start: 0, end: 0 },
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            },
        ],
        chunks: vec![
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                order: 0,
                text: first.to_string(),
            },
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(11),
                order: 1,
                text: second.to_string(),
            },
        ],
        cards: vec![CreateCardInput {
            node_id: maestria_domain::StructureNodeId::new(10),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            card_id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: card_body.to_string(),
            security: None,
        }],
    })
}

#[test]
fn full_ingestion_flow_detection_then_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Preflight
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    let output = state.apply_input(DomainInput::ParserCompleted(two_chunk_parser_result(
        "first chunk",
        "second chunk",
        "Document summary",
    )?))?;

    // Artifact references chunks and cards
    let artifact = &state.artifacts[&ArtifactId::new(1)];
    assert_eq!(artifact.chunk_ids.len(), 2);
    assert_eq!(artifact.card_ids.len(), 1);
    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    assert!(state.chunks.contains_key(&ChunkId::new(11)));
    assert!(state.cards.contains_key(&CardId::new(20)));

    // Events include: 2 chunks, 1 card, 1 ArtifactParsed
    let chunk_reg_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ChunkRegistered { .. }))
        .count();
    let card_created_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::CardCreated { .. }))
        .count();
    let parsed_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count();
    assert_eq!(chunk_reg_count, 2, "two chunk events");
    assert_eq!(card_created_count, 1, "one card event");
    assert_eq!(parsed_count, 1, "one artifact-parsed terminal event");

    Ok(())
}

#[test]
fn ingestion_replay_from_detection_only() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = vec![DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    })];
    let (state, events, _effects) = replay_inputs(&inputs)?;
    // Detection is a pure preflight — no persisted events or artifact.
    assert!(events.is_empty(), "detection emits no persisted events");
    assert!(state.artifacts.is_empty());
    assert!(state.pending_artifacts.contains_key(&ArtifactId::new(1)));
    // Replay from an empty event log produces a fresh empty state (pending
    // metadata is in-memory only and is not reconstructed from events).
    let replayed = replay_events(&events)?;
    assert!(replayed.pending_artifacts.is_empty());
    Ok(())
}

#[test]
fn ingestion_replay_full_flow_reconstructs_state() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = vec![
        DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: String::new(),
            source_bytes: Vec::new(),
            content_hash: "sha256:abc".to_string(),
        }),
        DomainInput::ParserCompleted(ParserResult {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            artifact_version_id: ArtifactVersionId::new(1),
            content_hash: fixtures::test_content_hash()?,
            tree_root_id: Some(StructureNodeId::new(10)),
            tree_nodes: vec![
                fixtures::tree_root_node(StructureNodeId::new(10)),
                maestria_domain::StructureNode {
                    id: StructureNodeId::new(11),
                    parent_id: Some(StructureNodeId::new(10)),
                    sibling_id: None,
                    node_type: maestria_domain::StructureNodeType::Paragraph,
                    source_range: ContentRange { start: 0, end: 0 },
                    page: None,
                    section_path: vec![],
                    parser_generation: "test".to_string(),
                    schema_generation: "1".to_string(),
                    language: None,
                },
            ],
            chunks: vec![
                RegisterChunkInput {
                    source_span: maestria_domain::SourceSpan::TextSpan {
                        start_line: 1,
                        end_line: 1,
                    },
                    representations: vec![],
                    chunk_id: ChunkId::new(10),
                    artifact_id: ArtifactId::new(1),
                    node_id: StructureNodeId::new(10),
                    order: 0,
                    text: "chunk one".to_string(),
                },
                RegisterChunkInput {
                    source_span: maestria_domain::SourceSpan::TextSpan {
                        start_line: 1,
                        end_line: 1,
                    },
                    representations: vec![],
                    chunk_id: ChunkId::new(11),
                    artifact_id: ArtifactId::new(1),
                    node_id: StructureNodeId::new(11),
                    order: 1,
                    text: "chunk two".to_string(),
                },
            ],
            cards: vec![CreateCardInput {
                node_id: maestria_domain::StructureNodeId::new(10),
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                card_id: CardId::new(20),
                artifact_id: ArtifactId::new(1),
                title: "Summary".to_string(),
                body: "Parsed doc".to_string(),
                security: None,
            }],
        }),
    ];
    let (state, events, _effects) = replay_inputs(&inputs)?;

    assert_eq!(state.artifacts.len(), 1);
    assert_eq!(state.chunks.len(), 2);
    assert_eq!(state.cards.len(), 1);

    // Replay events reconstructs the same artifacts, chunks, cards, and event log.
    // document_trees and artifact_versions are populated only during replay.
    let replayed = replay_events(&events)?;
    assert_eq!(state.artifacts, replayed.artifacts, "artifacts match");
    assert_eq!(state.chunks, replayed.chunks, "chunks match");
    assert_eq!(state.cards, replayed.cards, "cards match");
    assert_eq!(state.event_log, replayed.event_log, "event log matches");
    assert_eq!(
        state.pending_full_text, replayed.pending_full_text,
        "pending full text matches"
    );
    assert_eq!(
        state.parsed_artifact_ids, replayed.parsed_artifact_ids,
        "parsed artifact ids matches"
    );
    // document_trees and artifact_versions are populated during replay but not
    // during normal processing — verify they are set on the replayed state.
    assert!(
        replayed.document_trees.contains_key(&ArtifactId::new(1)),
        "replay populates document_trees"
    );
    assert!(
        replayed.artifact_versions.contains_key(&ArtifactId::new(1)),
        "replay populates artifact_versions"
    );
    Ok(())
}

#[test]
fn ingestion_replay_rejects_duplicate_detection_events() -> Result<(), Box<dyn std::error::Error>> {
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            security: maestria_domain::SecurityMetadata::default(),
        },
    };
    let mut state = KernelState::new();
    state.apply_event(event.clone())?;

    let duplicate = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            security: maestria_domain::SecurityMetadata::default(),
        },
    };
    let err = require_error(
        state.apply_event(duplicate),
        "duplicate artifact registration in replay must error",
    )?;
    assert!(matches!(
        err,
        DomainError::DuplicateId {
            kind: "artifact",
            id: 1,
        }
    ));
    Ok(())
}

#[test]
fn changed_hash_commits_new_pending_index_at_parse() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();

    // First detection + parse cycle
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:aaa".to_string(),
    }))?;
    let output1 = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    }))?;
    // First parse commits ArtifactRegistered + PendingIndex with hash aaa
    assert!(output1.events.iter().any(|e| matches!(&e.event,
        DomainEvent::PendingIndex { content_hash, .. } if content_hash == "sha256:aaa"
    )));
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].content_hash,
        Some("sha256:aaa".to_string())
    );

    // Mark as indexed so the second detection is not a no-op due to indexed-same-hash.
    if let Some(artifact) = state.artifacts.get_mut(&ArtifactId::new(1)) {
        artifact.index_status = IndexStatus::Indexed;
    }

    // Second detection + parse with different hash
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![4, 5, 6],
        content_hash: "sha256:bbb".to_string(),
    }))?;
    let output2 = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    }))?;
    // Second parse commits a new PendingIndex with hash bbb
    assert!(output2.events.iter().any(|e| matches!(&e.event,
        DomainEvent::PendingIndex { content_hash, .. } if content_hash == "sha256:bbb"
    )));
    assert!(
        !output2
            .events
            .iter()
            .any(|e| matches!(&e.event, DomainEvent::ArtifactRegistered { .. })),
        "ArtifactRegistered not duplicated"
    );
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].content_hash,
        Some("sha256:bbb".to_string())
    );
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );
    Ok(())
}

#[test]
fn pending_detection_not_treated_as_unchanged() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // First detection → pure preflight, no persisted events
    let output1 = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    assert_eq!(output1.events.len(), 0, "detection emits no events");
    assert!(state.pending_artifacts.contains_key(&ArtifactId::new(1)));
    assert!(
        output1
            .effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::ParseArtifact(_)))
    );

    // Second detection with same hash — still re-drives the pipeline because the
    // artifact has not been committed + indexed. Detection is never a no-op for
    // un-committed or un-indexed artifacts.
    let output2 = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;

    assert_eq!(output2.events.len(), 0, "detection still emits no events");
    assert!(
        output2
            .effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::ParseArtifact(_))),
        "should re-emit parse effect to re-drive the pipeline"
    );
    assert!(state.pending_artifacts.contains_key(&ArtifactId::new(1)));
    Ok(())
}

#[test]
fn full_text_index_partial_feedback() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Detect
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    // Parse with two chunks
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                order: 0,
                text: "a".to_string(),
            },
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(11),
                order: 1,
                text: "b".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;
    // Both chunks are pending
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );

    // Full-text index completes for one chunk
    let output = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;

    // Chunk 10 removed from pending
    assert!(!state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));
    // Still Pending — not all chunks indexed
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );
    // Emitted FullTextIndexed but not ArtifactIndexed
    assert_eq!(output.events.len(), 1);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::FullTextIndexed {
            chunk_id: ChunkId(10),
            ..
        }
    ));
    Ok(())
}

#[test]
fn full_text_index_final_feedback_emits_artifact_indexed() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = KernelState::new();
    // Detect + Parse with one chunk
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
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
            text: "a".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // Record evidence so evidence completeness check passes
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: evidence_id_for(ArtifactId::new(1), 0),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/notes.md".to_string(),
            range: ContentRange { start: 0, end: 1 },
            content_hash: "sha256:abc".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: "a".to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));

    // Full-text index completes for the only chunk
    let output = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;

    // All chunks indexed → ArtifactIndexed
    assert!(state.pending_full_text.is_empty());
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert_eq!(output.events.len(), 2);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::FullTextIndexed { .. }
    ));
    assert!(matches!(
        &output.events[1].event,
        DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId(1)
        }
    ));
    Ok(())
}

#[test]
fn duplicate_full_text_index_feedback_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Detect + Parse
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
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
            text: "a".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // Record evidence so evidence completeness check passes
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: evidence_id_for(ArtifactId::new(1), 0),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/notes.md".to_string(),
            range: ContentRange { start: 0, end: 1 },
            content_hash: "sha256:abc".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: "a".to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;

    // First feedback
    let output1 = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    assert_eq!(output1.events.len(), 2); // FullTextIndexed + ArtifactIndexed

    // Second feedback for same chunk — idempotent
    let output2 = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    assert_eq!(
        output2.events.len(),
        0,
        "duplicate feedback must be idempotent"
    );
    assert_eq!(output2.effects.len(), 0);
    Ok(())
}

#[test]
fn replay_reconstructs_pending_and_indexed_state() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(parser_result_two_chunks()?))?;

    // Record two evidences (one per chunk) so terminalization gate passes
    record_file_evidence(&mut state, 0, 0, 1, "a")?;
    record_file_evidence(&mut state, 1, 1, 2, "b")?;

    index_chunk(&mut state, 10)?;
    index_chunk(&mut state, 11)?;

    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert!(state.pending_full_text.is_empty());

    replay_assert_indexed_parity(&state)?;
    Ok(())
}
