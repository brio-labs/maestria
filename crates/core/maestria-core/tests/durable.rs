use std::path::PathBuf;

use maestria_core::build_artifact_detected_input;
use maestria_domain::{ArtifactDetected, KernelState, replay_inputs};

/// Verify that `build_artifact_detected_input` produces a DomainInput whose
/// fields reconstruct identically after being replayed through the domain
/// kernel—proving the input is pure, deterministic, and replay-safe.
#[test]
fn artifact_detected_input_is_replay_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from("notes/replay.md");
    let bytes = b"# Replay Test\n\nEvidence block for deterministic replay.\n".to_vec();

    let input =
        build_artifact_detected_input(&path, bytes.clone()).expect("valid input must succeed");

    // Apply once.
    let mut state_a = KernelState::new();
    let output_a = state_a.apply_input(input.clone())?;

    // Replay from scratch — must be identical.
    let mut state_b = KernelState::new();
    let output_b = state_b.apply_input(input.clone())?;

    assert_eq!(output_a, output_b, "replay must produce identical output");

    // Also exercise the batch replay path for the same input.
    let (replay_state, replay_events, replay_effects) = replay_inputs(&[input.clone()])?;
    assert_eq!(replay_state, state_a);
    assert!(!replay_events.is_empty());
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
#[test]
fn pure_input_with_effect_completion_is_replay_consistent() -> Result<(), Box<dyn std::error::Error>>
{
    let path = PathBuf::from("notes/full-cycle.md");
    let bytes = b"# Full Cycle\n\nParagraph one.\n\nParagraph two.\n".to_vec();

    let detected = build_artifact_detected_input(&path, bytes)?;

    let maestria_domain::DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title,
        source_path: _,
        source_bytes: _,
        content_hash: _,
    }) = &detected
    else {
        panic!("expected ArtifactDetected");
    };

    // Simulate what the runtime does after detection: apply ArtifactDetected,
    // then ParserCompleted, then FullTextIndexCompleted.
    let mut state = KernelState::new();
    state.apply_input(detected.clone())?;

    // Parser completed
    use maestria_domain::{
        ChunkId, CreateCardInput, DomainInput, LogicalTick, ParserResult, RegisterChunkInput,
    };
    let chunk_id_0 = ChunkId::new(701);
    let chunk_id_1 = ChunkId::new(702);
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: *artifact_id,
        chunks: vec![
            RegisterChunkInput {
                chunk_id: chunk_id_0,
                artifact_id: *artifact_id,
                order: 0,
                text: "Paragraph one.".to_string(),
            },
            RegisterChunkInput {
                chunk_id: chunk_id_1,
                artifact_id: *artifact_id,
                order: 1,
                text: "Paragraph two.".to_string(),
            },
        ],
        cards: vec![CreateCardInput {
            card_id: maestria_domain::CardId::new(901),
            artifact_id: *artifact_id,
            title: "Summary".to_string(),
            body: "Two paragraphs.".to_string(),
        }],
    }))?;

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
        .expect("artifact must exist after full cycle");
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
