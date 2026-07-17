use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

fn require_error<T, E>(
    result: Result<T, E>,
    message: &str,
) -> Result<E, Box<dyn std::error::Error>> {
    match result {
        Ok(_) => Err(std::io::Error::other(message).into()),
        Err(error) => Ok(error),
    }
}

// ── Deterministic evidence validation and identity ────────────────

#[test]
fn malformed_deterministic_existing_replaced_by_valid_retry()
-> Result<(), Box<dyn std::error::Error>> {
    // When an existing record at a deterministic ID is malformed
    // (e.g. legacy/corrupt), a valid incoming FileSpan with snapshot
    // and correct content_hash must replace it.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    let det_id = evidence_id_for(art_id, 0);
    // Set up artifact + chunk so the ID is deterministic.
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
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
            artifact_id: art_id,
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "hello".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // First, inject malformed evidence (CommandOutput — wrong kind).
    // We bypass RecordEvidence's deterministic check by inserting directly.
    state.evidences.insert(
        det_id,
        Evidence {
            id: det_id,
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(1),
                stream: OutputStream::Stdout,
                blob: BlobId::new(99),
            },
            excerpt: "old".to_string(),
            observed_at: LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        },
    );
    if let Some(artifact) = state.artifacts.get_mut(&art_id) {
        artifact.evidence_ids.insert(det_id);
    }
    assert!(state.evidences.contains_key(&det_id));

    // Now retry with valid deterministic evidence.
    let output = state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: det_id,
        artifact_id: art_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/test.md".to_string(),
            range: ContentRange { start: 0, end: 1 },
            content_hash: "sha256:abc".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: "hello".to_string(),
        observed_at: LogicalTick::new(2),
        security: None,
    }))?;
    assert!(
        output
            .events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::EvidenceRecorded { .. })),
        "replacement must emit EvidenceRecorded"
    );
    // Verify the evidence was replaced.
    let ev = state.evidences.get(&det_id).ok_or("evidence must exist")?;
    assert!(
        matches!(ev.kind, EvidenceKind::FileSpan { .. }),
        "replaced evidence must be FileSpan"
    );
    assert_eq!(ev.excerpt, "hello");
    Ok(())
}

#[test]
fn valid_deterministic_duplicate_still_rejected() -> Result<(), Box<dyn std::error::Error>> {
    // A valid existing record at a deterministic ID with different
    // fields must still return DuplicateId — idempotency is preserved.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    let det_id = evidence_id_for(art_id, 0);
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
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
            artifact_id: art_id,
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "hello".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // Insert valid deterministic evidence.
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: det_id,
        artifact_id: art_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/test.md".to_string(),
            range: ContentRange { start: 0, end: 1 },
            content_hash: "sha256:abc".to_string(),
            snapshot: Some(BlobId::new(42)),
        },
        excerpt: "hello".to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;
    // Retry with different excerpt — must be rejected.
    let err = require_error(
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: det_id,
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/test.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:abc".to_string(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: "different".to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        })),
        "valid duplicate mismatch must error",
    )?;
    assert!(
        matches!(err, DomainError::DuplicateId { kind, id } if kind == "evidence" && id == det_id.value()),
        "expected DuplicateId, got {:?}",
        err
    );
    Ok(())
}

#[test]
fn deterministic_cross_owner_rejected() -> Result<(), Box<dyn std::error::Error>> {
    // Evidence at a deterministic ID derived from artifact A cannot
    // be recorded under artifact B.
    let mut state = KernelState::new();
    let art_a = ArtifactId::new(1);
    let art_b = ArtifactId::new(2);
    let det_id = evidence_id_for(art_a, 0); // deterministic for artifact A
    // Set up artifact A with a chunk.
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: art_a,
        title: "A".to_string(),
        source_path: "/tmp/a.md".to_string(),
        source_bytes: vec![1],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_a,
        title: "A".to_string(),
        source_path: "/tmp/a.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(1),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_a,
        artifact_version_id: ArtifactVersionId::new(art_a.value()),
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
            artifact_id: art_a,
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "a".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // Set up artifact B so MissingArtifact doesn't fire first.
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: art_b,
        title: "B".to_string(),
        security: None,
    }))?;
    // Try to record under artifact B with artifact A's deterministic ID.
    let err = require_error(
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: det_id,
            artifact_id: art_b, // cross-owner
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/a.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:abc".to_string(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: "a".to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        })),
        "cross-owner deterministic evidence must be rejected",
    )?;
    assert!(
        matches!(err, DomainError::MalformedDeterministicEvidence { .. }),
        "expected MalformedDeterministicEvidence, got {:?}",
        err
    );
    Ok(())
}

#[test]
fn malformed_deterministic_non_filespan_is_rejected_at_record()
-> Result<(), Box<dyn std::error::Error>> {
    // Regression: CommandOutput evidence at a deterministic evidence ID
    // (matching a chunk) is rejected at RecordEvidence time because
    // deterministic evidence must be source-backed FileSpan.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
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
            artifact_id: art_id,
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "hello".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // CommandOutput at deterministic ID — MUST be rejected.
    let err = require_error(
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: evidence_id_for(art_id, 0),
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(1),
                stream: OutputStream::Stdout,
                blob: BlobId::new(99),
            },
            excerpt: "out".to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        })),
        "CommandOutput at deterministic evidence ID must be rejected",
    )?;
    assert!(
        matches!(err, DomainError::MalformedDeterministicEvidence { .. }),
        "expected MalformedDeterministicEvidence, got {:?}",
        err
    );
    // No evidence was inserted — state is unchanged.
    assert!(
        !state.evidences.contains_key(&evidence_id_for(art_id, 0)),
        "malformed evidence must not be inserted"
    );
    Ok(())
}

#[test]
fn malformed_deterministic_filespan_without_snapshot_is_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    // Regression: FileSpan evidence at a deterministic evidence ID
    // (matching a chunk) must have snapshot: Some. Missing snapshot
    // is rejected at RecordEvidence time.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
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
            artifact_id: art_id,
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "hello".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // FileSpan with snapshot: None — MUST be rejected.
    let err = require_error(
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: evidence_id_for(art_id, 0),
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/test.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:abc".to_string(),
                snapshot: None,
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        })),
        "FileSpan without snapshot at deterministic ID must be rejected",
    )?;
    assert!(
        matches!(err, DomainError::MalformedDeterministicEvidence { .. }),
        "expected MalformedDeterministicEvidence, got {:?}",
        err
    );
    assert!(
        !state.evidences.contains_key(&evidence_id_for(art_id, 0)),
        "malformed evidence must not be inserted"
    );
    Ok(())
}

#[test]
fn malformed_deterministic_wrong_content_hash_is_rejected() -> Result<(), Box<dyn std::error::Error>>
{
    // Regression: FileSpan evidence at a deterministic evidence ID
    // must have content_hash matching the artifact's recorded hash.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
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
            artifact_id: art_id,
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "hello".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    // FileSpan with wrong content_hash — MUST be rejected.
    let err = require_error(
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: evidence_id_for(art_id, 0),
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/test.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:WRONG".to_string(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        })),
        "FileSpan with wrong content_hash at deterministic ID must be rejected",
    )?;
    assert!(
        matches!(err, DomainError::MalformedDeterministicEvidence { .. }),
        "expected MalformedDeterministicEvidence, got {:?}",
        err
    );
    assert!(
        !state.evidences.contains_key(&evidence_id_for(art_id, 0)),
        "malformed evidence must not be inserted"
    );
    Ok(())
}

fn malformed_to_valid_replacement_events(
    art_id: ArtifactId,
    chunk_id: ChunkId,
    ev_id: EvidenceId,
) -> Vec<DomainEventEnvelope> {
    vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: art_id,
                title: "Test".to_string(),
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: art_id,
                title: "Test".to_string(),
                source_path: "/tmp/test.md".to_string(),
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: art_id,
                content_hash: "sha256:abc".to_string(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::ArtifactParsed {
                status: maestria_domain::ParseStatus::Parsed,
                artifact_id: art_id,
                chunks_added: 1,
            },
        },
        DomainEventEnvelope {
            id: EventId::new(5),
            sequence: SequenceNumber::new(5),
            event: DomainEvent::ChunkRegistered {
                node_id: maestria_domain::StructureNodeId::new(1),
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id,
                artifact_id: art_id,
                order: 0,
                text: "hello".to_string(),
            },
        },
        // Malformed evidence record (CommandOutput, not FileSpan).
        DomainEventEnvelope {
            id: EventId::new(6),
            sequence: SequenceNumber::new(6),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::CommandOutput {
                    harness_run: HarnessRunId::new(1),
                    stream: OutputStream::Stdout,
                    blob: BlobId::new(99),
                },
                excerpt: "old".to_string(),
                observed_at: LogicalTick::new(1),
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
        // Valid replacement (FileSpan with snapshot and correct hash).
        DomainEventEnvelope {
            id: EventId::new(7),
            sequence: SequenceNumber::new(7),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/tmp/test.md".to_string(),
                    range: ContentRange { start: 0, end: 1 },
                    content_hash: "sha256:abc".to_string(),
                    snapshot: Some(BlobId::new(42)),
                },
                excerpt: "hello".to_string(),
                observed_at: LogicalTick::new(2),
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
    ]
}
#[test]
fn replay_events_malformed_to_valid_evidence_replacement() -> Result<(), Box<dyn std::error::Error>>
{
    // When replaying an event log that contains a malformed deterministic
    // evidence record followed by a valid replacement at the same ID,
    // replay_events must replace the malformed record and clean up
    // reverse links instead of rejecting the second event as a duplicate.
    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let ev_id = evidence_id_for(art_id, 0);
    let events = malformed_to_valid_replacement_events(art_id, chunk_id, ev_id);
    let replayed = replay_events(&events)?;

    // Verify the evidence was replaced, not duplicated.
    assert_eq!(replayed.evidences.len(), 1);
    let ev = replayed
        .evidences
        .get(&ev_id)
        .ok_or("evidence must exist after replacement")?;
    assert!(
        matches!(ev.kind, EvidenceKind::FileSpan { .. }),
        "replaced evidence must be FileSpan"
    );
    assert_eq!(ev.excerpt, "hello");
    assert_eq!(ev.observed_at, LogicalTick::new(2));
    // Reverse link must point to the correct artifact.
    let artifact = &replayed.artifacts[&art_id];
    assert!(
        artifact.evidence_ids.contains(&ev_id),
        "artifact must link to replaced evidence"
    );
    Ok(())
}

fn valid_duplicate_evidence_events() -> Vec<DomainEventEnvelope> {
    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let ev_id = evidence_id_for(art_id, 0);
    vec![
        DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: art_id,
                title: "Test".to_string(),
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::ParserStarted {
                artifact_id: art_id,
                title: "Test".to_string(),
                source_path: "/tmp/test.md".to_string(),
                content_hash: "sha256:abc".to_string(),
                blob_id: BlobId::new(42),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::PendingIndex {
                artifact_id: art_id,
                content_hash: "sha256:abc".to_string(),
            },
        },
        DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::ArtifactParsed {
                status: maestria_domain::ParseStatus::Parsed,
                artifact_id: art_id,
                chunks_added: 1,
            },
        },
        DomainEventEnvelope {
            id: EventId::new(5),
            sequence: SequenceNumber::new(5),
            event: DomainEvent::ChunkRegistered {
                node_id: maestria_domain::StructureNodeId::new(1),
                source_span: maestria_domain::SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                representations: vec![],
                chunk_id,
                artifact_id: art_id,
                order: 0,
                text: "hello".to_string(),
            },
        },
        // Valid evidence.
        DomainEventEnvelope {
            id: EventId::new(6),
            sequence: SequenceNumber::new(6),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/tmp/test.md".to_string(),
                    range: ContentRange { start: 0, end: 1 },
                    content_hash: "sha256:abc".to_string(),
                    snapshot: Some(BlobId::new(42)),
                },
                excerpt: "hello".to_string(),
                observed_at: LogicalTick::new(1),
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
        // Different valid evidence at same ID — must error.
        DomainEventEnvelope {
            id: EventId::new(7),
            sequence: SequenceNumber::new(7),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: ev_id,
                artifact_id: art_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "/tmp/test.md".to_string(),
                    range: ContentRange { start: 1, end: 2 },
                    content_hash: "sha256:abc".to_string(),
                    snapshot: Some(BlobId::new(42)),
                },
                excerpt: "different".to_string(),
                observed_at: LogicalTick::new(2),
                security: maestria_domain::SecurityMetadata::default(),
            },
        },
    ]
}

#[test]
fn replay_events_valid_duplicate_evidence_still_errors() -> Result<(), Box<dyn std::error::Error>> {
    // A valid deterministic evidence record followed by a *different*
    // valid record at the same ID must still fail replay.
    let events = valid_duplicate_evidence_events();
    let err = require_error(replay_events(&events), "duplicate valid evidence must fail")?;
    assert!(
        matches!(err, DomainError::DuplicateId { kind, .. } if kind == "evidence"),
        "expected DuplicateId evidence error, got {:?}",
        err
    );
    Ok(())
}
