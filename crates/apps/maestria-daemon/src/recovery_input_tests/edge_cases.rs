use crate::recovery_inputs;
use maestria_domain::{
    ArtifactDetected, ArtifactId, ArtifactVersionId, BlobId, ChunkId, ContentHash, ContentRange,
    DomainInput, KernelState, ParseStatus, ParserResult, ParserStarted, RegisterChunkInput,
    SourceSpan, StructureNode, StructureNodeId, StructureNodeType,
};

#[test]
fn recovery_inputs_empty_when_nothing_pending() -> Result<(), Box<dyn std::error::Error>> {
    let state = KernelState::new();
    let recovery = recovery_inputs(&state);
    assert!(recovery.resume_parsers.is_empty());
    assert!(recovery.start_full_text.is_empty());
    Ok(())
}

#[test]
fn recovery_inputs_derives_both_kinds_from_state() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let artifact_a = ArtifactId::new(1);
    let artifact_b = ArtifactId::new(2);

    // artifact_a: has pending parser (crashed mid-parse)
    state.pending_parsers.insert(
        artifact_a,
        ParserStarted {
            artifact_id: artifact_a,
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(100),
        },
    );

    // artifact_b: has pending chunks but no pending parser
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: artifact_b,
        title: "b.md".to_string(),
        source_path: "/tmp/b.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:def".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: artifact_b,
        artifact_version_id: ArtifactVersionId::new(artifact_b.value()),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        status: ParseStatus::Parsed,
        tree_root_id: Some(StructureNodeId::new(20)),
        tree_nodes: vec![StructureNode {
            id: StructureNodeId::new(20),
            parent_id: None,
            sibling_id: None,
            node_type: StructureNodeType::Document,
            source_range: ContentRange { start: 0, end: 0 },
            page: None,
            section_path: vec![],
            parser_generation: "test".to_string(),
            schema_generation: "1".to_string(),
            language: None,
        }],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(20),
            artifact_id: artifact_b,
            node_id: StructureNodeId::new(20),
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            order: 0,
            text: "text".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    let recovery = recovery_inputs(&state);

    assert_eq!(
        recovery.resume_parsers.len(),
        1,
        "one ResumeParser for artifact_a"
    );
    assert_eq!(
        recovery.start_full_text.len(),
        1,
        "one StartFullTextIndex for artifact_b"
    );

    // Verify ordering: resume parsers are from pending_parsers
    match &recovery.resume_parsers[0] {
        DomainInput::ResumeParser(r) => assert_eq!(r.artifact_id, artifact_a),
        other => return Err(format!("expected ResumeParser, got {other:?}").into()),
    }

    // Verify full-text inputs skip parser-pending artifacts
    match &recovery.start_full_text[0] {
        DomainInput::StartFullTextIndex(s) => assert_eq!(s.artifact_id, artifact_b),
        other => return Err(format!("expected StartFullTextIndex, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn recovery_inputs_excludes_parser_pending_from_full_text() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(1);

    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "doc.md".to_string(),
        source_path: "/tmp/doc.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id,
        artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        status: ParseStatus::Parsed,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![StructureNode {
            id: StructureNodeId::new(10),
            parent_id: None,
            sibling_id: None,
            node_type: StructureNodeType::Document,
            source_range: ContentRange { start: 0, end: 0 },
            page: None,
            section_path: vec![],
            parser_generation: "test".to_string(),
            schema_generation: "1".to_string(),
            language: None,
        }],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id,
            node_id: StructureNodeId::new(10),
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            order: 0,
            text: "text".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // Now simulate re-ingestion crash: ParserStarted replayed
    state.pending_parsers.insert(
        artifact_id,
        ParserStarted {
            artifact_id,
            title: "doc.md".to_string(),
            source_path: "/tmp/doc.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(100),
        },
    );

    let recovery = recovery_inputs(&state);

    assert_eq!(
        recovery.resume_parsers.len(),
        1,
        "ResumeParser for the pending parser"
    );
    assert!(
        recovery.start_full_text.is_empty(),
        "StartFullTextIndex must be empty when the only pending artifact has a pending parser"
    );
    Ok(())
}
