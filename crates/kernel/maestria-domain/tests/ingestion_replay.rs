use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/fixtures.rs"]
mod fixtures;
#[path = "common/ingestion.rs"]
mod ingestion_helpers;

use assertions::require_error;
use ingestion_helpers::{
    index_chunk, parser_result_two_chunks, record_file_evidence, replay_assert_indexed_parity,
};

// ── Replay behavior for ingestion pipeline ────────────────────────

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
