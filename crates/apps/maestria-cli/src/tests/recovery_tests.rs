use maestria_domain::{
    ArtifactDetected, ArtifactId, ArtifactVersionId, BlobId, ChunkId, ContentHash, DomainInput,
    IndexStatus, KernelState, ParseStatus, ParserResult, ParserStarted, RegisterChunkInput,
    SourceSpan, StartFullTextIndex, StructureNodeId,
};

/// Verify that `recovery_inputs` — as called by `index_path` before
/// `build_runtime` — correctly derives `ResumeParser` inputs from
/// pending parsers and `StartFullTextIndex` inputs from pending
/// full-text chunks, excluding artifacts that have a pending parser
/// (whose resumed flow owns index dispatch).  This regression guards
/// against the bug where CLI `index_path` started a fresh runtime but
/// never queued durable pending work, causing an equal-hash artifact
/// to be skipped and the CLI to wait until timeout.
#[test]
fn index_path_recovery_derives_pending_inputs_with_correct_filter()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let artifact_a = ArtifactId::new(1); // has pending parser
    let artifact_b = ArtifactId::new(2); // has pending chunks only

    // artifact_a: ParserStarted replayed (pending parser)
    state.pending_parsers.insert(
        artifact_a,
        ParserStarted {
            artifact_id: artifact_a,
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: "sha256:aaa".to_string(),
            blob_id: BlobId::new(100),
        },
    );

    // artifact_b: ParserCompleted created chunks but indexing not finished
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: artifact_b,
        title: "b.md".to_string(),
        source_path: "/tmp/b.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:bbb".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: artifact_b,
        artifact_version_id: ArtifactVersionId::new(artifact_b.value()),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        status: ParseStatus::Parsed,
        tree_root_id: Some(StructureNodeId::new(20)),
        tree_nodes: vec![maestria_domain::StructureNode {
            id: maestria_domain::StructureNodeId::new(20),
            parent_id: None,
            sibling_id: None,
            node_type: maestria_domain::StructureNodeType::Document,
            source_range: maestria_domain::ContentRange { start: 0, end: 0 },
            page: None,
            section_path: vec![],
            parser_generation: "test".to_string(),
            schema_generation: "1".to_string(),
            language: None,
        }],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(20),
            artifact_id: artifact_b,
            node_id: maestria_domain::StructureNodeId::new(20),
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            order: 0,
            text: "hello".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    let recovery = maestria_daemon::recovery_inputs(&state);

    // artifact_a must be in resume_parsers
    assert_eq!(
        recovery.resume_parsers.len(),
        1,
        "one ResumeParser for artifact_a"
    );
    assert!(
        matches!(
            &recovery.resume_parsers[0],
            DomainInput::ResumeParser(r) if r.artifact_id == artifact_a
        ),
        "recovery.resume_parsers[0] must be ResumeParser for artifact_a"
    );

    // artifact_a must NOT appear in start_full_text (parser flow owns indexing)
    assert!(
        recovery
            .start_full_text
            .iter()
            .all(|input| !matches!(input, DomainInput::StartFullTextIndex(s) if s.artifact_id == artifact_a)),
        "artifact_a must be excluded from start_full_text"
    );

    // artifact_b must be in start_full_text
    assert_eq!(
        recovery.start_full_text.len(),
        1,
        "one StartFullTextIndex for artifact_b"
    );
    assert!(
        matches!(
            &recovery.start_full_text[0],
            DomainInput::StartFullTextIndex(s) if s.artifact_id == artifact_b
        ),
        "recovery.start_full_text[0] must be StartFullTextIndex for artifact_b"
    );
    Ok(())
}

#[test]
fn index_path_recovery_empty_when_no_pending_work() -> Result<(), Box<dyn std::error::Error>> {
    let state = KernelState::new();
    let recovery = maestria_daemon::recovery_inputs(&state);
    assert!(recovery.resume_parsers.is_empty());
    assert!(recovery.start_full_text.is_empty());
    assert!(recovery.run_validations.is_empty());
    Ok(())
}

/// Verify that recovery artifact IDs are correctly extracted from
/// both `resume_parsers` and `start_full_text` vectors, covering
/// the two input kinds the drain loop must await.
#[test]
fn recovery_artifact_ids_covers_both_input_kinds() -> Result<(), Box<dyn std::error::Error>> {
    let recovery = maestria_daemon::RecoveryInputs {
        resume_parsers: vec![DomainInput::ResumeParser(ParserStarted {
            artifact_id: ArtifactId::new(10),
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: "sha256:aa".to_string(),
            blob_id: BlobId::new(100),
        })],
        start_full_text: vec![
            DomainInput::StartFullTextIndex(StartFullTextIndex {
                artifact_id: ArtifactId::new(20),
            }),
            DomainInput::StartFullTextIndex(StartFullTextIndex {
                artifact_id: ArtifactId::new(30),
            }),
        ],
        run_validations: Vec::new(),
    };

    let ids: Vec<ArtifactId> = {
        let resume_ids = recovery.resume_parsers.iter().filter_map(|input| {
            if let DomainInput::ResumeParser(r) = input {
                Some(r.artifact_id)
            } else {
                None
            }
        });
        let ft_ids = recovery.start_full_text.iter().filter_map(|input| {
            if let DomainInput::StartFullTextIndex(s) = input {
                Some(s.artifact_id)
            } else {
                None
            }
        });
        resume_ids.chain(ft_ids).collect()
    };

    assert_eq!(ids.len(), 3, "three recovery artifact IDs");
    assert!(ids.contains(&ArtifactId::new(10)), "resume parser ID 10");
    assert!(ids.contains(&ArtifactId::new(20)), "start full-text ID 20");
    assert!(ids.contains(&ArtifactId::new(30)), "start full-text ID 30");
    Ok(())
}

/// Verify that the recovery drain predicate — "all artifact IDs are
/// Indexed" — correctly distinguishes terminal from non-terminal
/// states.  This is the same check the drain loop uses each poll
/// iteration before shutdown.
#[test]
fn recovery_drain_all_indexed_predicate() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let id_a = ArtifactId::new(1);
    let id_b = ArtifactId::new(2);

    // Register both artifacts through the full pipeline:
    // ArtifactDetected → ParserCompleted so they land in state.artifacts.
    for &id in &[id_a, id_b] {
        state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: id,
            title: format!("{id}.md"),
            source_path: format!("/tmp/{id}.md"),
            source_bytes: vec![id.value() as u8],
            content_hash: format!("sha256:{id}"),
        }))?;
        state.apply_input(DomainInput::ParserCompleted(ParserResult {
            artifact_id: id,
            artifact_version_id: ArtifactVersionId::new(id.value()),
            content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
            status: ParseStatus::Parsed,
            tree_root_id: Some(maestria_domain::StructureNodeId::new(0)),
            tree_nodes: vec![maestria_domain::StructureNode {
                id: maestria_domain::StructureNodeId::new(0),
                parent_id: None,
                sibling_id: None,
                node_type: maestria_domain::StructureNodeType::Document,
                source_range: maestria_domain::ContentRange { start: 0, end: 0 },
                page: None,
                section_path: vec![],
                parser_generation: "test".to_string(),
                schema_generation: "1".to_string(),
                language: None,
            }],
            chunks: Vec::new(),
            cards: Vec::new(),
        }))?;
    }

    // Predicate: all indexed
    let all_indexed = |state: &KernelState, ids: &[ArtifactId]| -> bool {
        ids.iter().all(|id| {
            state
                .artifacts
                .get(id)
                .is_some_and(|a| a.index_status == IndexStatus::Indexed)
        })
    };

    // Neither indexed → false.
    assert!(!all_indexed(&state, &[id_a, id_b]));

    // Mark only id_a as Indexed → still false.
    state
        .artifacts
        .get_mut(&id_a)
        .ok_or_else(|| std::io::Error::other("artifact A missing"))?
        .index_status = IndexStatus::Indexed;
    assert!(!all_indexed(&state, &[id_a, id_b]));

    // Mark id_b as Indexed → true.
    state
        .artifacts
        .get_mut(&id_b)
        .ok_or_else(|| std::io::Error::other("artifact B missing"))?
        .index_status = IndexStatus::Indexed;
    assert!(all_indexed(&state, &[id_a, id_b]));

    // Empty list is vacuously true.
    assert!(all_indexed(&state, &[]));
    Ok(())
}

/// Verify that `maestria_daemon::reconcile_projections` is callable
/// from the CLI context and succeeds on a fresh in-memory store.
/// The daemon crate's own `projection_recovery_tests` cover the full
/// repair contract (missing artifact, chunk, card, evidence rows).
/// This smoke test guards the CLI `index_path` path: the call site
/// compiles against the CLI's dependency set and completes without
/// error on a realistic kernel state.
#[test]
fn index_path_reconcile_projections_succeeds_in_cli_context()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(100);

    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "cli-repair.md".to_string(),
        source_path: "/tmp/cli-repair.md".to_string(),
        source_bytes: vec![9, 8, 7],
        content_hash: "sha256:cli".to_string(),
    }))?;

    let store = maestria_storage_sqlite::SqliteStore::in_memory()?;

    // Reconcile with the store — should succeed.
    maestria_daemon::reconcile_projections(&state, &store)?;

    // Reconcile again — must be idempotent.
    maestria_daemon::reconcile_projections(&state, &store)?;
    Ok(())
}
