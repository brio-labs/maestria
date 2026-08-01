use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/fixtures.rs"]
mod fixtures;

use assertions::require_error;

// ── Deterministic evidence validation at record time ──────────────

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
        content_hash: fixtures::test_content_hash()?.as_str().to_owned(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: fixtures::test_content_hash()?.as_str().to_owned(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))?],
        chunks: vec![RegisterChunkInput {
            source_span: maestria_domain::SourceSpan::text_span(1, 1)?,
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
            range: LineRange::new(1, 1)?,
            snapshot: SnapshotRef::new(BlobId::new(42), fixtures::test_content_hash()?),
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
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(BlobId::new(42), fixtures::test_content_hash()?),
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
        content_hash: fixtures::test_content_hash()?.as_str().to_owned(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_a,
        title: "A".to_string(),
        source_path: "/tmp/a.md".to_string(),
        content_hash: fixtures::test_content_hash()?.as_str().to_owned(),
        blob_id: BlobId::new(1),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_a,
        artifact_version_id: ArtifactVersionId::new(art_a.value()),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))?],
        chunks: vec![RegisterChunkInput {
            source_span: maestria_domain::SourceSpan::text_span(1, 1)?,
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
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(BlobId::new(42), fixtures::test_content_hash()?),
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
        content_hash: fixtures::test_content_hash()?.as_str().to_owned(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: art_id,
        title: "Test".to_string(),
        source_path: "/tmp/test.md".to_string(),
        content_hash: fixtures::test_content_hash()?.as_str().to_owned(),
        blob_id: BlobId::new(42),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: art_id,
        artifact_version_id: ArtifactVersionId::new(art_id.value()),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(10)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))?],
        chunks: vec![RegisterChunkInput {
            source_span: maestria_domain::SourceSpan::text_span(1, 1)?,
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
fn zero_based_deterministic_filespan_range_is_rejected_at_typed_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    // A FileSpan can no longer represent either a missing snapshot or the
    // legacy zero-based range used by this fixture. The range constructor
    // rejects the invalid locator before EvidenceKind can be constructed.
    let range = LineRange::new(0, 1);
    let state = KernelState::new();
    assert!(
        !state
            .evidences
            .contains_key(&evidence_id_for(ArtifactId::new(1), 0)),
        "boundary-rejected evidence must not be inserted"
    );
    assert!(
        matches!(range, Err(LineRangeError::StartMustBePositive)),
        "legacy zero-based FileSpan range must be rejected"
    );
    Ok(())
}

#[test]
fn malformed_deterministic_content_hash_is_rejected_at_typed_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    // A malformed snapshot hash is rejected before EvidenceKind can be
    // constructed, so impossible deterministic evidence never reaches state.
    let hash = ContentHash::new("sha256:WRONG".to_string());
    let state = KernelState::new();
    assert!(
        !state
            .evidences
            .contains_key(&evidence_id_for(ArtifactId::new(1), 0)),
        "boundary-rejected evidence must not be inserted"
    );
    assert!(
        matches!(hash, Err(SearchCompatibilityError::InvalidContentHash(_))),
        "malformed snapshot content hash must be rejected"
    );
    Ok(())
}

#[test]
fn content_hash_requires_lowercase_hex_digits() -> Result<(), Box<dyn std::error::Error>> {
    let uppercase = ContentHash::new("sha256:".to_owned() + &"A".repeat(64));
    assert!(
        matches!(
            uppercase,
            Err(SearchCompatibilityError::InvalidContentHash(_))
        ),
        "uppercase hexadecimal digits must be rejected"
    );

    let lowercase = ContentHash::new("sha256:".to_owned() + &"a".repeat(64))?;
    assert_eq!(lowercase.as_str(), format!("sha256:{}", "a".repeat(64)));
    Ok(())
}

#[test]
fn live_chunk_registration_rejects_preexisting_malformed_deterministic_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(1);
    let evidence_id = evidence_id_for(artifact_id, 0);
    let content_hash = fixtures::test_content_hash()?;
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id,
        title: "Test".to_string(),
        security: None,
    }))?;
    let artifact = state
        .artifacts
        .get_mut(&artifact_id)
        .ok_or("registered artifact must exist")?;
    artifact.content_hash = Some(content_hash.as_str().to_owned());
    state.evidences.insert(
        evidence_id,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(1),
                stream: OutputStream::Stdout,
                blob: BlobId::new(99),
            },
            excerpt: "output".to_string(),
            observed_at: LogicalTick::new(1),
            security: SecurityMetadata::default(),
        },
    );
    let before = state.clone();

    let error = require_error(
        state.apply_input(DomainInput::RegisterChunk(RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id,
            node_id: StructureNodeId::new(10),
            source_span: SourceSpan::text_span(1, 1)?,
            representations: vec![],
            order: 0,
            text: "hello".to_string(),
        })),
        "malformed deterministic evidence must block chunk registration",
    )?;
    assert!(matches!(
        error,
        DomainError::MalformedDeterministicEvidence {
            evidence_id: rejected_id,
            ..
        } if rejected_id == evidence_id
    ));
    assert_eq!(state, before, "failed live chunk registration is atomic");
    Ok(())
}
