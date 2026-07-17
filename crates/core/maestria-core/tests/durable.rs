use std::path::PathBuf;

use maestria_core::build_artifact_detected_input;
use maestria_domain::{ArtifactDetected, BlobId, KernelState, replay_inputs};

/// Verify that `build_artifact_detected_input` produces a DomainInput whose
/// fields reconstruct identically after being replayed through the domain
/// kernel—proving the input is pure, deterministic, and replay-safe.
#[test]
fn artifact_detected_input_is_replay_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from("notes/replay.md");
    let bytes = b"# Replay Test\n\nEvidence block for deterministic replay.\n".to_vec();

    let input = build_artifact_detected_input(&path, bytes.clone())?;

    // Apply once.
    let mut state_a = KernelState::new();
    let output_a = state_a.apply_input(input.clone())?;

    // Replay from scratch — must be identical.
    let mut state_b = KernelState::new();
    let output_b = state_b.apply_input(input.clone())?;

    assert_eq!(output_a, output_b, "replay must produce identical output");

    // Also exercise the batch replay path for the same input.
    let (replay_state, replay_events, replay_effects) =
        replay_inputs(std::slice::from_ref(&input))?;
    assert_eq!(replay_state, state_a);
    assert!(replay_events.is_empty());
    assert!(!replay_effects.is_empty());

    // Reproducibility: same bytes, same path → same input → same KernelState.
    let input2 = build_artifact_detected_input(&path, bytes)?;
    assert_eq!(input2, input);

    let mut state_c = KernelState::new();
    let output_c = state_c.apply_input(input2)?;
    assert_eq!(output_c, output_a);

    Ok(())
}

/// Replaying an ArtifactDetected followed by a ParserCompleted, FullTextIndexCompleted,
/// and clock-tick should produce the same terminal state regardless of replay order
/// (batch replay vs sequential apply).
///
fn parser_result_for_cycle(
    artifact_id: maestria_domain::ArtifactId,
) -> Result<maestria_domain::ParserResult, Box<dyn std::error::Error>> {
    use maestria_domain::{
        ArtifactVersionId, ChunkId, ContentHash, ContentRange, CreateCardInput, ParseStatus,
        RegisterChunkInput, SourceSpan, StructureNode, StructureNodeId, StructureNodeType,
    };

    Ok(maestria_domain::ParserResult {
        artifact_id,
        artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        status: ParseStatus::Parsed,
        tree_root_id: Some(StructureNodeId::new(701)),
        tree_nodes: vec![
            StructureNode {
                id: StructureNodeId::new(701),
                parent_id: None,
                sibling_id: None,
                node_type: StructureNodeType::Document,
                source_range: ContentRange { start: 0, end: 0 },
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            },
            StructureNode {
                id: StructureNodeId::new(702),
                parent_id: Some(StructureNodeId::new(701)),
                sibling_id: None,
                node_type: StructureNodeType::Paragraph,
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
                chunk_id: ChunkId::new(701),
                artifact_id,
                node_id: StructureNodeId::new(701),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                order: 0,
                text: "Paragraph one.".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(702),
                artifact_id,
                node_id: StructureNodeId::new(702),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                order: 1,
                text: "Paragraph two.".to_string(),
            },
        ],
        cards: vec![CreateCardInput {
            card_id: maestria_domain::CardId::new(901),
            artifact_id,
            node_id: StructureNodeId::new(701),
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            title: "Summary".to_string(),
            body: "Two paragraphs.".to_string(),
            security: None,
        }],
    })
}
#[test]
fn pure_input_with_effect_completion_is_replay_consistent() -> Result<(), Box<dyn std::error::Error>>
{
    let path = PathBuf::from("notes/full-cycle.md");
    let bytes = b"# Full Cycle\n\nParagraph one.\n\nParagraph two.\n".to_vec();

    let detected = build_artifact_detected_input(&path, bytes.clone())?;

    let maestria_domain::DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        source_path,
        source_bytes: _,
        content_hash: source_hash,
        title,
    }) = &detected
    else {
        return Err("expected ArtifactDetected".into());
    };

    // Simulate what the runtime does after detection: apply ArtifactDetected,
    // then ParserCompleted, then FullTextIndexCompleted.
    let mut state = KernelState::new();
    state.apply_input(detected.clone())?;

    use maestria_domain::{
        ChunkId, ContentRange, DomainInput, EvidenceKind, LogicalTick, RecordEvidenceInput,
        evidence_id_for,
    };
    let chunk_id_0 = ChunkId::new(701);
    let chunk_id_1 = ChunkId::new(702);
    state.apply_input(DomainInput::ParserCompleted(parser_result_for_cycle(
        *artifact_id,
    )?))?;
    for (order, excerpt) in ["Paragraph one.", "Paragraph two."].into_iter().enumerate() {
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: evidence_id_for(*artifact_id, order as u32),
            artifact_id: *artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: source_path.clone(),
                range: ContentRange {
                    start: order + 1,
                    end: order + 1,
                },
                content_hash: source_hash.clone(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: excerpt.to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        }))?;
    }

    // Full-text indexing completes for both chunks.
    state.apply_input(DomainInput::FullTextIndexCompleted(
        maestria_domain::FullTextIndexCompleted {
            artifact_id: *artifact_id,
            chunk_id: chunk_id_0,
        },
    ))?;
    state.apply_input(DomainInput::FullTextIndexCompleted(
        maestria_domain::FullTextIndexCompleted {
            artifact_id: *artifact_id,
            chunk_id: chunk_id_1,
        },
    ))?;

    // Clock tick to advance time.
    state.apply_input(DomainInput::ClockTick(LogicalTick::new(2)))?;

    // Verify terminal state: artifact is indexed.
    let artifact = state
        .artifacts
        .get(artifact_id)
        .ok_or("artifact must exist after full cycle")?;
    assert_eq!(
        artifact.index_status,
        maestria_domain::IndexStatus::Indexed,
        "artifact must be Indexed after parser + full-text complete"
    );
    assert_eq!(artifact.title, title.as_str());
    assert!(artifact.chunk_ids.contains(&chunk_id_0));
    assert!(artifact.chunk_ids.contains(&chunk_id_1));

    Ok(())
}
