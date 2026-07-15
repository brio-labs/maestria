use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

// ── Crash recovery and resume flows ───────────────────────────────

#[test]
fn resume_after_crash_replays_and_completes() -> Result<(), DomainError> {
    // Simulate: ParserStarted event persisted, then crash.
    // On restart, replay reconstructs pending_parsers, daemon sends ResumeParser,
    // runtime re-parses, ParserCompleted cleans up.
    let events = vec![DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    }];

    let mut state = replay_events(&events)?;
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // Daemon finds pending entry and sends ResumeParser
    let output = state.apply_input(DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::ParseArtifact(req)
            if req.source_blob == Some(BlobId::new(42))
    ));

    // Parser completes — clean up pending_parsers and create artifact
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "recovered chunk".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers retained after ParserCompleted on resume; cleared only at ArtifactIndexed"
    );
    assert!(state.artifacts.contains_key(&ArtifactId::new(1)));
    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    Ok(())
}

#[test]
fn parser_completed_cleanup_idempotent_on_resume_retry() -> Result<(), DomainError> {
    // On resume, ParserCompleted may be sent multiple times.
    // Each time it must be idempotent — pending_parsers removed once.
    let events = vec![DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: String::new(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    }];
    let mut state = replay_events(&events)?;
    state.apply_input(DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;

    let result = ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "chunk".to_string(),
        }],
        cards: Vec::new(),
    };

    // First ParserCompleted
    state.apply_input(DomainInput::ParserCompleted(result.clone()))?;
    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers retained after first ParserCompleted"
    );

    // Second ParserCompleted (retry) — must not error, must remain clean.
    // The tree was committed during the first pass, so no duplicate
    // lifecycle event is emitted.
    let output = state.apply_input(DomainInput::ParserCompleted(result))?;
    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers retained after retry"
    );
    assert!(output.events.is_empty(), "retry emits no duplicate events");
    Ok(())
}

#[test]
fn crash_before_evidence_pending_parsers_survives_for_resume() -> Result<(), DomainError> {
    // Simulate: parser completes, emits ArtifactParsed + chunks, but
    // ArtifactIndexed never fires (crash). On resume, replay reconstructs
    // pending_parsers and the parser re-runs idempotently.
    let mut state = KernelState::new();

    // Full ingestion: detection → parser started → parser completed.
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:aaa".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:aaa".to_string(),
        blob_id: BlobId::new(42),
    }))?;

    let chunk_input = RegisterChunkInput {
        chunk_id: ChunkId::new(10),
        artifact_id: ArtifactId::new(1),
        node_id: StructureNodeId::new(10),
        order: 0,
        text: "hello".to_string(),
    };
    let card_input = CreateCardInput {
        card_id: CardId::new(20),
        artifact_id: ArtifactId::new(1),
        title: "Summary".to_string(),
        body: "body".to_string(),
    };

    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![chunk_input.clone()],
        cards: vec![card_input.clone()],
    }))?;

    // After ParserCompleted, pending_parsers still exists (not yet removed).
    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers survives ArtifactParsed — crash-before-evidence leaves it retryable"
    );

    // Simulate resume: re-run identical ParserCompleted (idempotent).
    let output_resume = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![chunk_input],
        cards: vec![card_input],
    }))?;

    // No new chunks/cards → duplicate should suppress ArtifactParsed.
    let parsed_on_resume = output_resume
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count();
    assert_eq!(
        parsed_on_resume, 0,
        "resume duplicate emits no ArtifactParsed"
    );

    // Record evidence so the terminal ArtifactIndexed evidence gate passes.
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: evidence_id_for(ArtifactId::new(1), 0),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/notes.md".to_string(),
            range: ContentRange { start: 0, end: 5 },
            content_hash: "sha256:aaa".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: "hello".to_string(),
        observed_at: LogicalTick::new(1),
    }))?;

    // Now simulate terminal indexing clearing pending_parsers.
    // Mark chunk as full-text indexed → all done → ArtifactIndexed.
    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;

    assert!(
        !state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactIndexed clears pending_parsers"
    );
    Ok(())
}

struct IngestArtifactSetup {
    art_id: ArtifactId,
    title: String,
    source_path: String,
    source_bytes: Vec<u8>,
    content_hash: String,
    blob_id: BlobId,
    chunks: Vec<RegisterChunkInput>,
}

fn ingest_artifact_full(
    state: &mut KernelState,
    setup: IngestArtifactSetup,
) -> Result<(), DomainError> {
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: setup.art_id,
        title: setup.title.clone(),
        source_path: setup.source_path.clone(),
        source_bytes: setup.source_bytes,
        content_hash: setup.content_hash.clone(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: setup.art_id,
        title: setup.title.clone(),
        source_path: setup.source_path.clone(),
        content_hash: setup.content_hash.clone(),
        blob_id: setup.blob_id,
    }))?;
    let _ = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: setup.art_id,
        artifact_version_id: ArtifactVersionId::new(setup.art_id.value()),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: setup.chunks,
        cards: Vec::new(),
    }))?;
    Ok(())
}

struct FileSpanEvidenceSetup {
    ev_id: EvidenceId,
    art_id: ArtifactId,
    path: String,
    range: ContentRange,
    content_hash: String,
    snapshot: Option<BlobId>,
    excerpt: String,
    tick: u64,
}

fn record_file_span_evidence(
    state: &mut KernelState,
    setup: FileSpanEvidenceSetup,
) -> Result<(), DomainError> {
    let _ = state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: setup.ev_id,
        artifact_id: setup.art_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: setup.path.clone(),
            range: setup.range,
            content_hash: setup.content_hash.clone(),
            snapshot: setup.snapshot,
        },
        excerpt: setup.excerpt.clone(),
        observed_at: LogicalTick::new(setup.tick),
    }))?;
    Ok(())
}

/// Applies FullTextIndexCompleted for a chunk and asserts the result was a
/// FullTextIndexed event for that chunk, leaving the artifact Pending.
fn index_chunk_and_assert_pending(
    state: &mut KernelState,
    art_id: ArtifactId,
    chunk_id: ChunkId,
    expected_chunk: u64,
) -> Result<(), DomainError> {
    let output = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: art_id,
            chunk_id,
        },
    ))?;
    assert_eq!(output.events.len(), 1);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::FullTextIndexed {
            chunk_id: ChunkId(chunk),
            ..
        } if *chunk == expected_chunk
    ));
    assert_eq!(state.artifacts[&art_id].index_status, IndexStatus::Pending);
    Ok(())
}

fn record_complete_evidence_and_terminalize(
    state: &mut KernelState,
    art_id: ArtifactId,
) -> Result<(), DomainError> {
    for (order, excerpt, range) in [
        (0, "a", ContentRange { start: 0, end: 1 }),
        (1, "b", ContentRange { start: 1, end: 2 }),
    ] {
        record_file_span_evidence(
            state,
            FileSpanEvidenceSetup {
                ev_id: evidence_id_for(art_id, order),
                art_id,
                path: "/tmp/notes.md".to_string(),
                range,
                content_hash: "sha256:abc".to_string(),
                snapshot: Some(BlobId::new(42)),
                excerpt: excerpt.to_string(),
                tick: 1,
            },
        )?;
    }
    let output = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: art_id,
    }))?;
    assert_eq!(state.artifacts[&art_id].index_status, IndexStatus::Indexed);
    assert!(!state.pending_parsers.contains_key(&art_id));
    assert!(
        output
            .events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::ArtifactIndexed { .. }))
    );
    Ok(())
}

#[test]
fn missing_evidence_keeps_artifact_pending_after_full_text_done() -> Result<(), DomainError> {
    // Regression: when all chunks are indexed but no evidence exists for the
    // deterministic evidence IDs, terminalization MUST be blocked. The artifact
    // stays Pending and pending_parsers survives so retry/resume can regenerate
    // evidence.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);

    // Full ingestion: detect → parser started → parser completed.
    ingest_artifact_full(
        &mut state,
        IngestArtifactSetup {
            art_id,
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            source_bytes: vec![1, 2, 3],
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
            chunks: vec![
                RegisterChunkInput {
                    chunk_id: ChunkId::new(10),
                    artifact_id: art_id,
                    node_id: StructureNodeId::new(10),
                    order: 0,
                    text: "a".to_string(),
                },
                RegisterChunkInput {
                    chunk_id: ChunkId::new(11),
                    artifact_id: art_id,
                    node_id: StructureNodeId::new(11),
                    order: 1,
                    text: "b".to_string(),
                },
            ],
        },
    )?;

    assert!(state.pending_parsers.contains_key(&art_id));
    assert_eq!(state.artifacts[&art_id].index_status, IndexStatus::Pending);

    // Index chunk 10 — still not all done.
    index_chunk_and_assert_pending(&mut state, art_id, ChunkId::new(10), 10)?;

    // Index chunk 11 — all chunks done, but ZERO evidence recorded.
    // Terminalization must be blocked.
    index_chunk_and_assert_pending(&mut state, art_id, ChunkId::new(11), 11)?;
    assert!(
        state.pending_parsers.contains_key(&art_id),
        "pending_parsers must survive when evidence is incomplete"
    );

    // Record evidence for the WRONG chunk order — evidence exists but maps
    // to order 99 (chunk doesn't exist for this order). Terminalization must
    // still be blocked.
    let wrong_ev_id = evidence_id_for(art_id, 99);
    record_file_span_evidence(
        &mut state,
        FileSpanEvidenceSetup {
            ev_id: wrong_ev_id,
            art_id,
            path: "/tmp/notes.md".to_string(),
            range: ContentRange { start: 0, end: 1 },
            content_hash: "sha256:abc".to_string(),
            snapshot: None,
            excerpt: "wrong".to_string(),
            tick: 1,
        },
    )?;

    // Still blocked — chunk[0] evidence missing (wrong ev points to order 99).
    let output3 = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: art_id,
    }))?;
    assert_eq!(
        state.artifacts[&art_id].index_status,
        IndexStatus::Pending,
        "wrong-order evidence must not satisfy terminalization gate"
    );
    assert!(
        !output3
            .events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::ArtifactIndexed { .. })),
        "no ArtifactIndexed when evidence IDs don't match chunk orders"
    );

    // Correct evidence completes the terminalization gate.
    record_complete_evidence_and_terminalize(&mut state, art_id)?;
    Ok(())
}
