use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

fn parser_result_with_multiple_chunks() -> ParserResult {
    ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
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
                text: "chunk a".to_string(),
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
                text: "chunk b".to_string(),
            },
            RegisterChunkInput {
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id: ChunkId::new(12),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(12),
                order: 2,
                text: "chunk c".to_string(),
            },
        ],
        cards: vec![
            CreateCardInput {
                node_id: StructureNodeId::new(10),
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                card_id: CardId::new(20),
                artifact_id: ArtifactId::new(1),
                title: "Card 1".to_string(),
                body: "Alpha".to_string(),
            },
            CreateCardInput {
                node_id: StructureNodeId::new(10),
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                card_id: CardId::new(21),
                artifact_id: ArtifactId::new(1),
                title: "Card 2".to_string(),
                body: "Beta".to_string(),
            },
        ],
    }
}

fn replay_assert_parity(state: &KernelState) -> Result<KernelState, DomainError> {
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state.artifacts, replayed.artifacts, "artifacts match");
    assert_eq!(state.chunks, replayed.chunks, "chunks match");
    assert_eq!(state.cards, replayed.cards, "cards match");
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
    Ok(replayed)
}

#[test]
fn replay_ingestion_flow_state_parity() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
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
            text: "content".to_string(),
        }],
        cards: Vec::new(),
    }))?;

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
    Ok(())
}

#[test]
fn replay_ingestion_flow_with_multiple_chunks() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Big Doc".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(
        parser_result_with_multiple_chunks(),
    ))?;

    let replayed = replay_assert_parity(&state)?;
    assert_eq!(replayed.chunks.len(), 3);
    assert_eq!(replayed.cards.len(), 2);
    assert_eq!(
        replayed
            .artifacts
            .get(&ArtifactId::new(1))
            .ok_or(DomainError::MissingArtifact {
                id: ArtifactId::new(1),
            })?
            .chunk_ids
            .len(),
        3
    );
    Ok(())
}

#[test]
fn replay_ingestion_detection_only() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Pending".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    // Detection is a pure preflight — no persisted events, no artifact in
    // state.artifacts. The pending metadata is in-memory only.
    assert!(state.event_log.is_empty(), "detection emits no events");
    assert!(state.artifacts.is_empty());
    assert!(state.pending_artifacts.contains_key(&ArtifactId::new(1)));
    assert!(
        replay_events(&state.event_log)?
            .pending_artifacts
            .is_empty()
    );
    Ok(())
}
#[test]
fn replay_ingestion_duplicate_chunk_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
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
            text: "unique".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // event_log now has: ArtifactRegistered (id=1), PendingIndex (id=2), ChunkRegistered (id=3), ArtifactParsed (id=4)
    let next_id = state.event_log.len() as u64 + 1;
    let duplicate_chunk = DomainEventEnvelope {
        id: EventId::new(next_id),
        sequence: SequenceNumber::new(next_id),
        event: DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "duplicate".to_string(),
        },
    };
    let err = state
        .apply_event(duplicate_chunk)
        .expect_err("duplicate chunk in replay must error");
    assert!(matches!(
        err,
        DomainError::DuplicateId {
            kind: "chunk",
            id: 10,
        }
    ));
    Ok(())
}

#[test]
fn replay_ingestion_parser_without_detection_rejected() -> Result<(), DomainError> {
    let next_id = 1u64;
    let orphan_artparsed = DomainEventEnvelope {
        id: EventId::new(next_id),
        sequence: SequenceNumber::new(next_id),
        event: DomainEvent::ArtifactParsed {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(99),
            chunks_added: 0,
        },
    };
    let mut state = KernelState::new();
    let err = state
        .apply_event(orphan_artparsed)
        .expect_err("ArtifactParsed without artifact must error");
    assert!(matches!(
        err,
        DomainError::MissingArtifact { id } if id == ArtifactId::new(99)
    ));
    assert!(state.event_log.is_empty());
    Ok(())
}

#[test]
fn replay_ingestion_orphan_chunk_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let orphan_chunk = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(1),
            artifact_id: ArtifactId::new(99),
            order: 0,
            text: "orphan".to_string(),
        },
    };
    let err = state
        .apply_event(orphan_chunk)
        .expect_err("ChunkRegistered without artifact must error");
    assert!(matches!(
        err,
        DomainError::MissingArtifact { id } if id == ArtifactId::new(99)
    ));
    Ok(())
}

#[test]
fn replay_full_text_indexed_rejects_mismatched_chunk_artifact() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    // Set up: artifact 1 owns chunk 10, artifact 2 is separate
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Artifact A".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(2),
            title: "Artifact B".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "a".to_string(),
        },
    })?;

    // FullTextIndexed mismatches: chunk 10 belongs to artifact 1, not artifact 2
    let err = state
        .apply_event(DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::FullTextIndexed {
                artifact_id: ArtifactId::new(2),
                chunk_id: ChunkId::new(10),
            },
        })
        .expect_err("mismatched chunk artifact must be rejected");

    assert!(matches!(
        err,
        DomainError::ArtifactMismatch {
            expected: ArtifactId(2),
            actual: ArtifactId(1),
        }
    ));
    Ok(())
}

#[test]
fn replay_artifact_indexed_rejects_pending_chunks() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    // Set up: artifact with a pending chunk
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Test".to_string(),
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
        event: DomainEvent::ChunkRegistered {
            node_id: maestria_domain::StructureNodeId::new(1),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "content".to_string(),
        },
    })?;

    // ArtifactIndexed when chunks are still pending must be rejected
    let err = state
        .apply_event(DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::ArtifactIndexed {
                artifact_id: ArtifactId::new(1),
            },
        })
        .expect_err("ArtifactIndexed with pending chunks must be rejected");

    assert!(matches!(
        err,
        DomainError::PendingChunksExist {
            artifact_id: ArtifactId(1),
        }
    ));
    Ok(())
}

#[test]
fn replay_parser_started_reconstructs_pending_parsers() -> Result<(), DomainError> {
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

    assert_eq!(state.pending_parsers.len(), 1);
    let pending = &state.pending_parsers[&ArtifactId::new(1)];
    assert_eq!(pending.title, "Notes");
    assert_eq!(pending.blob_id, BlobId::new(42));
    assert_eq!(pending.source_path, "/tmp/notes.md");
    assert_eq!(pending.content_hash, "sha256:abc");
    Ok(())
}

#[test]
fn replay_parser_started_multiple_entries() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Doc A".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: "sha256:aaa".to_string(),
            blob_id: BlobId::new(10),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(2),
            title: "Doc B".to_string(),
            source_path: "/tmp/b.md".to_string(),
            content_hash: "sha256:bbb".to_string(),
            blob_id: BlobId::new(20),
        },
    })?;

    assert_eq!(state.pending_parsers.len(), 2);
    assert_eq!(
        state.pending_parsers[&ArtifactId::new(1)].blob_id,
        BlobId::new(10)
    );
    assert_eq!(
        state.pending_parsers[&ArtifactId::new(2)].blob_id,
        BlobId::new(20)
    );
    Ok(())
}

#[test]
fn replay_artifact_parsed_retains_pending_parsers() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Full reconstruction: ArtifactRegistered → ParserStarted → ArtifactParsed
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
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // ArtifactParsed must NOT clear pending_parsers — only ArtifactIndexed does.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ArtifactParsed {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            chunks_added: 1,
        },
    })?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactParsed replay must NOT clean pending_parsers"
    );
    Ok(())
}

#[test]
fn replay_artifact_parsed_zero_chunks_retains_pending_parsers() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Full reconstruction: ArtifactRegistered → ParserStarted → ArtifactParsed(chunks_added=0)
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
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // ArtifactParsed with chunks_added=0 must NOT clean pending_parsers.
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
        "ArtifactParsed with chunks_added=0 must NOT clean pending_parsers on replay"
    );
    Ok(())
}

#[test]
fn replay_search_completed_preserves_pending_parsers() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Set up: artifact exists and parser is in-flight.
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
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // SearchCompleted arrives for the same artifact — must NOT clear pending_parsers.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::SearchCompleted {
            artifact_id: ArtifactId::new(1),
            cards_added: 3,
        },
    })?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "SearchCompleted must preserve pending_parsers for in-flight parser recovery"
    );
    assert_eq!(state.pending_parsers.len(), 1);
    Ok(())
}

#[test]
fn replay_parser_started_id_is_sequential() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // ParserStarted replayed with a gap in ID must be rejected.
    let err = state
        .apply_event(DomainEventEnvelope {
            id: EventId::new(5),
            sequence: SequenceNumber::new(5),
            event: DomainEvent::ParserStarted {
                artifact_id: ArtifactId::new(1),
                title: "Notes".to_string(),
                source_path: String::new(),
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(1),
            },
        })
        .expect_err("ParserStarted with non-sequential ID must error");
    assert!(matches!(
        err,
        DomainError::InvalidEventId {
            expected: 1,
            actual: 5
        }
    ));
    Ok(())
}

#[test]
fn replay_parser_started_via_convenience() -> Result<(), DomainError> {
    // Replay from event list: ParserStarted entry survives into reconstructed state.
    let events = vec![DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Doc".to_string(),
            source_path: "/tmp/doc.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(7),
        },
    }];
    let replayed = replay_events(&events)?;
    assert_eq!(replayed.pending_parsers.len(), 1);
    assert_eq!(
        replayed.pending_parsers[&ArtifactId::new(1)].blob_id,
        BlobId::new(7)
    );
    Ok(())
}
