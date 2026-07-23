use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

// ── Parser result state registration ──────────────────────────────

#[test]
fn parser_completed_registers_chunks_and_cards() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
        security: None,
    }))?;

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
        cards: vec![CreateCardInput {
            node_id: maestria_domain::StructureNodeId::new(10),
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            card_id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Parsed summary".to_string(),
            security: None,
        }],
    }))?;

    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    assert!(state.cards.contains_key(&CardId::new(20)));
    assert!(
        state
            .artifacts
            .get(&ArtifactId::new(1))
            .is_some_and(|artifact| artifact.chunk_ids.contains(&ChunkId::new(10))
                && artifact.card_ids.contains(&CardId::new(20)))
    );
    assert!(output.events.iter().any(|event| matches!(
        event.event,
        DomainEvent::CardCreated {
            node_id: _,
            source_span: maestria_domain::SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1
            },
            card_id: CardId(20),
            artifact_id: ArtifactId(1),
            ..
        }
    )));
    Ok(())
}
