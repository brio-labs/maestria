use super::*;
use maestria_domain::{
    ArtifactDetected, ArtifactId, ArtifactVersionId, BlobId, ChangeTaskStatusInput, ChunkId,
    ContentHash, ContentRange, DomainInput, KernelState, MaestriaEffect, OpenTaskInput,
    ParseStatus, ParserResult, ParserStarted, RegisterChunkInput, SourceSpan, StructureNode,
    StructureNodeId, StructureNodeType, TaskId,
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
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id,
                node_id: StructureNodeId::new(10),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                order: 0,
                text: "chunk a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id,
                node_id: StructureNodeId::new(11),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
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
        content_hash: "sha256:def".to_string(),
    }))?;

    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id,
        artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
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
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(20),
                artifact_id,
                node_id: StructureNodeId::new(20),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                order: 0,
                text: "hello".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(21),
                artifact_id,
                node_id: StructureNodeId::new(21),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
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
            content_hash: "sha256:abc".to_string(),
        }))?;

        state.apply_input(DomainInput::ParserCompleted(ParserResult {
            artifact_id,
            artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
            content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
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
                source_range: ContentRange { start: 0, end: 0 },
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
            content_hash: "sha256:abc".to_string(),
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

// ── recovery_inputs tests ──────────────────────────────────────────────

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

#[test]
fn pending_validations_derives_validation_for_validating_tasks_without_reports()
-> Result<(), maestria_domain::DomainError> {
    let mut state = KernelState::new();

    // Create a task that is in Validating state without a report
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(1),
        title: "Task 1".to_string(),
        artifact_id: None,
        priority: maestria_domain::TaskPriority::High,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(1),
        to: maestria_domain::TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(1),
        to: maestria_domain::TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(1),
        to: maestria_domain::TaskStatus::Validating,
    }))?;

    // Add a second task that is Validating but already has a report
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(2),
        title: "Task 2".to_string(),
        artifact_id: None,
        priority: maestria_domain::TaskPriority::Normal,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Validating,
    }))?;
    state.apply_input(DomainInput::RecordValidationReport(
        maestria_domain::RecordValidationReportInput {
            report_id: maestria_domain::ValidationReportId::new(1),
            task_id: Some(TaskId::new(2)),
            passed: true,
            warnings: vec![],
        },
    ))?;
    // Re-enter Validating after the first report; the historical report must
    // not satisfy the new validation cycle.
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(2),
        to: maestria_domain::TaskStatus::Validating,
    }))?;

    // Add a third task that is not Validating
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(3),
        title: "Task 3".to_string(),
        artifact_id: None,
        priority: maestria_domain::TaskPriority::Normal,
    }))?;

    let recovery = recovery_inputs(&state);

    assert_eq!(
        recovery.run_validations.len(),
        2,
        "Should recover both tasks without a report for their current validation cycle"
    );

    let validation_task_ids: Vec<_> = recovery
        .run_validations
        .iter()
        .filter_map(|input| match input {
            DomainInput::RequestTaskValidation(request) => Some(request.task_id),
            _ => None,
        })
        .collect();
    assert_eq!(validation_task_ids, vec![TaskId::new(1), TaskId::new(2)]);

    Ok(())
}
