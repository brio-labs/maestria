use crate::pending_start_full_text;
use maestria_domain::{
    ArtifactDetected, ArtifactId, ArtifactVersionId, BlobId, ChunkId, ContentRange, DomainInput,
    KernelState, MaestriaEffect, ParseStatus, ParserResult, ParserStarted, RegisterChunkInput,
    SourceSpan, StructureNode, StructureNodeId, StructureNodeType,
};

#[test]
fn pending_start_full_text_groups_by_artifact() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(1);

    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "test.md".to_string(),
        source_path: "/tmp/test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: maestria_test_support::content_hash(10)?,
    }))?;

    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id,
        artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
        content_hash: maestria_test_support::content_hash(0)?,
        status: ParseStatus::Parsed,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![StructureNode {
            id: StructureNodeId::new(10),
            parent_id: None,
            sibling_id: None,
            node_type: StructureNodeType::Document,
            source_range: ContentRange::new(0, 0)?,
            page: None,
            section_path: vec![],
            parser_generation: "test".to_string(),
            schema_generation: "1".to_string(),
            language: None,
        }],
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id,
                node_id: StructureNodeId::new(10),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                order: 0,
                text: "chunk a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id,
                node_id: StructureNodeId::new(11),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                order: 1,
                text: "chunk b".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;

    assert_eq!(state.pending_full_text.len(), 2);

    let inputs = pending_start_full_text(&state);
    assert_eq!(
        inputs.len(),
        1,
        "should produce one StartFullTextIndex input per artifact"
    );

    match &inputs[0] {
        DomainInput::StartFullTextIndex(start) => {
            assert_eq!(start.artifact_id, artifact_id);
        }
        other => return Err(format!("expected StartFullTextIndex, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn pending_start_full_text_resumes_indexing_without_reparse()
-> Result<(), Box<dyn std::error::Error>> {
    // pending_start_full_text produces StartFullTextIndex inputs that
    // emit full-text and vector effects without re-parsing source bytes.

    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(1);

    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "notes.md".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: maestria_test_support::content_hash(13)?,
    }))?;

    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id,
        artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
        content_hash: maestria_test_support::content_hash(0)?,
        status: ParseStatus::Parsed,
        tree_root_id: Some(StructureNodeId::new(20)),
        tree_nodes: vec![StructureNode {
            id: StructureNodeId::new(20),
            parent_id: None,
            sibling_id: None,
            node_type: StructureNodeType::Document,
            source_range: ContentRange::new(0, 0)?,
            page: None,
            section_path: vec![],
            parser_generation: "test".to_string(),
            schema_generation: "1".to_string(),
            language: None,
        }],
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(20),
                artifact_id,
                node_id: StructureNodeId::new(20),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                order: 0,
                text: "hello".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(21),
                artifact_id,
                node_id: StructureNodeId::new(21),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                order: 1,
                text: "world".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;

    assert_eq!(state.pending_full_text.len(), 2);
    let parser_full_text_effects = output
        .effects
        .iter()
        .filter(|effect| matches!(effect, MaestriaEffect::IndexFullText(_)))
        .count();
    let parser_vector_effects = output
        .effects
        .iter()
        .filter(|effect| matches!(effect, MaestriaEffect::IndexVector(_)))
        .count();
    assert_eq!(parser_full_text_effects, 0);
    assert_eq!(parser_vector_effects, 0);

    let event_count_before = state.event_log.len();

    // Simulate restart: build pending inputs and apply to the same state
    let pending_inputs = pending_start_full_text(&state);
    assert_eq!(pending_inputs.len(), 1);

    let restart_output = state.apply_input(pending_inputs[0].clone())?;
    // StartFullTextIndex emits full-text and vector effects but no new events.
    let event_count_after = state.event_log.len();
    assert_eq!(
        event_count_after, event_count_before,
        "StartFullTextIndex must not produce duplicate events"
    );
    let restart_full_text_effects = restart_output
        .effects
        .iter()
        .filter(|effect| matches!(effect, MaestriaEffect::IndexFullText(_)))
        .count();
    let restart_vector_effects = restart_output
        .effects
        .iter()
        .filter(|effect| matches!(effect, MaestriaEffect::IndexVector(_)))
        .count();
    assert_eq!(restart_full_text_effects, 2);
    assert_eq!(restart_vector_effects, 2);

    assert_eq!(state.pending_full_text.len(), 2);
    Ok(())
}

#[test]
fn pending_start_full_text_empty_when_nothing_pending() -> Result<(), Box<dyn std::error::Error>> {
    let state = KernelState::new();
    let inputs = pending_start_full_text(&state);
    assert!(inputs.is_empty());
    Ok(())
}

#[test]
fn pending_start_full_text_skips_orphan_chunk_ids() -> Result<(), Box<dyn std::error::Error>> {
    // If pending_full_text references a chunk_id not in state.chunks,
    // the helper should silently skip it.
    let mut state = KernelState::new();
    state.pending_full_text.insert(ChunkId::new(999));

    let inputs = pending_start_full_text(&state);
    assert!(inputs.is_empty(), "orphan chunk ids should be skipped");
    Ok(())
}

#[test]
fn pending_start_full_text_excludes_pending_parser_artifacts()
-> Result<(), Box<dyn std::error::Error>> {
    // Regression: artifacts with pending parser metadata must not
    // receive a StartFullTextIndex during recovery — the resumed
    // parser flow owns completion, evidence, and index ordering and
    // emits its own StartFullTextIndex afterward.  Issuing a separate
    // StartFullTextIndex here could make chunks terminal before
    // resumed evidence is recorded.

    let mut state = KernelState::new();
    let artifact_a = ArtifactId::new(1);
    let artifact_b = ArtifactId::new(2);

    // Set up both artifacts with chunks via the normal domain flow so
    // pending_full_text is populated.
    for (artifact_id, title) in [(artifact_a, "a.md"), (artifact_b, "b.md")] {
        state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: title.to_string(),
            source_path: format!("/tmp/{title}"),
            source_bytes: vec![1, 2, 3],
            content_hash: maestria_test_support::content_hash(10)?,
        }))?;

        state.apply_input(DomainInput::ParserCompleted(ParserResult {
            artifact_id,
            artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
            content_hash: maestria_test_support::content_hash(0)?,
            status: ParseStatus::Parsed,
            tree_root_id: Some(StructureNodeId::new(if artifact_id == artifact_a {
                10
            } else {
                20
            })),
            tree_nodes: vec![StructureNode {
                id: StructureNodeId::new(if artifact_id == artifact_a { 10 } else { 20 }),
                parent_id: None,
                sibling_id: None,
                node_type: StructureNodeType::Document,
                source_range: ContentRange::new(0, 0)?,
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            }],
            chunks: vec![RegisterChunkInput {
                chunk_id: ChunkId::new(if artifact_id == artifact_a { 10 } else { 20 }),
                artifact_id,
                node_id: StructureNodeId::new(if artifact_id == artifact_a { 10 } else { 20 }),
                source_span: SourceSpan::text_span(1, 1)?,
                representations: vec![],
                order: 0,
                text: "text".to_string(),
            }],
            cards: Vec::new(),
        }))?;
    }

    // After ParserCompleted, pending_parsers is empty.  Simulate a
    // re-ingestion crash: artifact_a was re-ingested (ParserStarted
    // replayed, pending_parsers set) but the process crashed before
    // ParserCompleted.  Old chunks from the prior parse still have
    // pending_full_text entries.
    state.pending_parsers.insert(
        artifact_a,
        ParserStarted {
            artifact_id: artifact_a,
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: maestria_test_support::content_hash(10)?,
            blob_id: BlobId::new(100),
        },
    );

    assert!(
        state.pending_full_text.len() >= 2,
        "both artifacts have pending chunks"
    );
    assert!(
        state.pending_parsers.contains_key(&artifact_a),
        "artifact_a has a pending parser"
    );
    assert!(
        !state.pending_parsers.contains_key(&artifact_b),
        "artifact_b has no pending parser"
    );

    let inputs = pending_start_full_text(&state);

    // Only artifact_b receives StartFullTextIndex.
    // artifact_a is excluded because the resumed parser flow will
    // handle completion, evidence, and its own index dispatch.
    assert_eq!(
        inputs.len(),
        1,
        "only artifact_b should get StartFullTextIndex"
    );
    match &inputs[0] {
        DomainInput::StartFullTextIndex(start) => {
            assert_eq!(
                start.artifact_id, artifact_b,
                "artifact_b gets StartFullTextIndex (no pending parser)"
            );
        }
        other => return Err(format!("expected StartFullTextIndex, got {other:?}").into()),
    }
    Ok(())
}
