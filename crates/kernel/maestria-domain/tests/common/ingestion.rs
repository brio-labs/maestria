use maestria_domain::*;

/// Build a [`ParserResult`] with exactly two chunks and no cards.
pub fn parser_result_two_chunks() -> Result<ParserResult, Box<dyn std::error::Error>> {
    Ok(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![StructureNode {
            id: StructureNodeId::new(10),
            parent_id: None,
            sibling_id: None,
            node_type: maestria_domain::StructureNodeType::Document,
            source_range: ContentRange { start: 0, end: 0 },
            page: None,
            section_path: vec![],
            parser_generation: "test".to_string(),
            schema_generation: "test".to_string(),
            language: None,
        }],
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
    })
}
/// Record deterministic [`FileSpan`] evidence against artifact 1.
pub fn record_file_evidence(
    state: &mut KernelState,
    order: u32,
    start: usize,
    end: usize,
    excerpt: &str,
) -> Result<(), DomainError> {
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: evidence_id_for(ArtifactId::new(1), order),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/notes.md".to_string(),
            range: ContentRange { start, end },
            content_hash: "sha256:abc".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: excerpt.to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;
    Ok(())
}

/// Mark a chunk as fully-text-indexed.
pub fn index_chunk(state: &mut KernelState, chunk_id: u64) -> Result<(), DomainError> {
    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(chunk_id),
        },
    ))?;
    Ok(())
}

/// Assert that replaying `state.event_log` yields the same indexed state.
pub fn replay_assert_indexed_parity(state: &KernelState) -> Result<(), DomainError> {
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
    assert_eq!(
        replayed.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert!(replayed.pending_full_text.is_empty());
    Ok(())
}
