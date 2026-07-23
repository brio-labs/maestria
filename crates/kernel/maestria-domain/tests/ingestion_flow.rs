use maestria_domain::*;

fn two_chunk_parser_result(
    first: &str,
    second: &str,
    card_body: &str,
) -> Result<ParserResult, Box<dyn std::error::Error>> {
    Ok(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![
            StructureNode {
                id: StructureNodeId::new(10),
                parent_id: None,
                sibling_id: None,
                node_type: maestria_domain::StructureNodeType::Document,
                source_range: ContentRange { start: 0, end: 0 },
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            },
            StructureNode {
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

// ── Full ingestion flow: detection → parsing ──────────────────────

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
