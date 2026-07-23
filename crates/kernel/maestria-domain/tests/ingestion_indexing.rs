use maestria_domain::*;

#[path = "common/fixtures.rs"]
mod fixtures;

// ── Full-text indexing feedback ───────────────────────────────────

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
