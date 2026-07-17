use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

#[test]
fn parser_started_identical_metadata_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let ps = ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:aaa".to_string(),
        blob_id: BlobId::new(42),
    };
    let out1 = state.apply_input(DomainInput::ParserStarted(ps.clone()))?;
    assert!(
        out1.events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::ParserStarted { .. }))
    );
    assert!(
        out1.effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::PersistEvent { .. }))
    );

    let out2 = state.apply_input(DomainInput::ParserStarted(ps))?;
    assert!(out2.events.is_empty());
    assert!(out2.effects.is_empty());
    Ok(())
}

#[test]
fn parser_started_differing_metadata_emits_replacement() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:aaa".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    let out = state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:bbb".to_string(),
        blob_id: BlobId::new(99),
    }))?;
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::ParserStarted { .. }))
    );
    assert!(
        out.effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::PersistEvent { .. }))
    );
    let stored = &state.pending_parsers[&ArtifactId::new(1)];
    assert_eq!(stored.content_hash, "sha256:bbb");
    assert_eq!(stored.blob_id, BlobId::new(99));
    Ok(())
}

#[test]
fn artifact_detected_with_active_pending_parser_is_noop() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:aaa".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    let out = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:aaa".to_string(),
    }))?;
    assert!(out.effects.is_empty());
    Ok(())
}

#[test]
fn artifact_detected_different_hash_with_pending_parser_proceeds()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:aaa".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    let out = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        source_bytes: vec![4, 5, 6],
        content_hash: "sha256:bbb".to_string(),
    }))?;
    assert!(
        out.effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::ParseArtifact(..)))
    );
    Ok(())
}

#[test]
fn parser_completed_does_not_emit_index_effects() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
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
        ],
        cards: Vec::new(),
    }))?;
    assert_eq!(
        output
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
            .count(),
        0
    );
    assert_eq!(
        output
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexVector(_)))
            .count(),
        0
    );
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));
    Ok(())
}
