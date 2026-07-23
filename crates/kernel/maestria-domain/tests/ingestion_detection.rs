use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/fixtures.rs"]
mod fixtures;

use assertions::require_error;

// ── Detection and preflight behavior ──────────────────────────────

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
