use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/fixtures.rs"]
mod fixtures;
#[path = "common/parser.rs"]
mod parser_helpers;

use assertions::require_error;
use parser_helpers::{count_card_events, count_chunk_events, count_parsed_events};

// ── ParserCompleted idempotency and validation ────────────────────

#[test]
fn parser_completed_duplicate_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        security: None,
    }))?;

    let parser_result = ParserResult {
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
            node_id: StructureNodeId::new(10),
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
fn parser_completed_rejects_mismatched_chunk() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        security: None,
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
            text: "first chunk".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // Second parse with same chunk_id but different text
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
                text: "different text".to_string(),
            }],
            cards: Vec::new(),
        })),
        "mismatched chunk must error",
    )?;

    assert!(matches!(err, DomainError::DuplicateId { kind, id: 10 } if kind == "chunk"));
    Ok(())
}

#[test]
fn parser_completed_rejects_mismatched_card() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
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
            security: None,
        }],
    }))?;

    // Second parse with same card_id but different body
    let err = require_error(
        state.apply_input(DomainInput::ParserCompleted(ParserResult {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            artifact_version_id: ArtifactVersionId::new(1),
            content_hash: fixtures::test_content_hash()?,
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
                security: None,
            }],
        })),
        "mismatched card must error",
    )?;

    assert!(matches!(err, DomainError::DuplicateId { kind, id: 20 } if kind == "card"));
    Ok(())
}

#[test]
fn parser_completed_first_zero_output_emits_artifact_parsed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        security: None,
    }))?;

    // First parse with zero chunks/cards must still emit ArtifactParsed.
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    }))?;

    let parsed_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count();
    assert_eq!(
        parsed_count, 1,
        "first zero-output parse emits ArtifactParsed"
    );
    Ok(())
}

#[test]
fn parser_completed_duplicate_zero_output_suppresses_artifact_parsed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        security: None,
    }))?;

    let empty_result = ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
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
            .any(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
    );

    // Second parse with same zero data — must suppress duplicate ArtifactParsed.
    let output2 = state.apply_input(DomainInput::ParserCompleted(empty_result))?;
    let parsed2 = output2
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count();
    assert_eq!(
        parsed2, 0,
        "duplicate zero-output parse suppresses ArtifactParsed"
    );
    Ok(())
}
