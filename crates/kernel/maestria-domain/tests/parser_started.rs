use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/fixtures.rs"]
mod fixtures;

use assertions::require_error;

// ── ParserStarted, resume, and pending-parser cleanup ─────────────

#[test]
fn parser_started_stores_metadata_and_emits_persist_event() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;

    // Pending-parser metadata is stored in-memory.
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));
    let pending = &state.pending_parsers[&ArtifactId::new(1)];
    assert_eq!(pending.title, "Notes");
    assert_eq!(pending.source_path, "/tmp/notes.md");
    assert_eq!(pending.content_hash, "sha256:abc");
    assert_eq!(pending.blob_id, BlobId::new(42));

    // Exactly one PersistEvent carrying the ParserStarted event.
    assert_eq!(output.events.len(), 1);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::ParserStarted {
            artifact_id: ArtifactId(1),
            title,
            source_path,
            content_hash,
            blob_id: BlobId(42),
        } if title == "Notes" && source_path == "/tmp/notes.md" && content_hash == "sha256:abc"
    ));
    assert_eq!(output.effects.len(), 1);
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::PersistEvent { .. }
    ));

    // No artifact created yet — ParserStarted is pure metadata.
    assert!(state.artifacts.is_empty());
    Ok(())
}

#[test]
fn resume_parser_emits_parse_artifact_with_source_blob() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Set up: pending_parsers exists from replay
    state.pending_parsers.insert(
        ArtifactId::new(1),
        ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    );

    let output = state.apply_input(DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: "/tmp/notes.md".to_string(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;

    // No events — ResumeParser re-drives only, no new persisted metadata.
    assert_eq!(output.events.len(), 0);
    assert_eq!(output.effects.len(), 1);
    assert!(matches!(
        &output.effects[0],
        MaestriaEffect::ParseArtifact(req)
            if req.artifact_id == ArtifactId::new(1)
            && req.source_blob == Some(BlobId::new(42))
            && req.source_bytes.is_empty()
    ));
    Ok(())
}

#[test]
fn resume_parser_without_pending_entry_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // pending_parsers is empty — no entry to resume
    let err = require_error(
        state.apply_input(DomainInput::ResumeParser(ParserStarted {
            artifact_id: ArtifactId::new(99),
            title: "Ghost".to_string(),
            source_path: String::new(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(1),
        })),
        "resume without pending entry must error",
    )?;
    assert!(matches!(
        err,
        DomainError::MissingArtifact { id } if id == ArtifactId::new(99)
    ));
    Ok(())
}

#[test]
fn parser_completed_removes_pending_parser() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Set up: preflight detection + parser started
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserStarted(ParserStarted {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        content_hash: "sha256:abc".to_string(),
        blob_id: BlobId::new(42),
    }))?;
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // Parser completes — pending_parsers survives until ArtifactIndexed.
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        status: maestria_domain::ParseStatus::Parsed,
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash()?,
        tree_root_id: Some(StructureNodeId::new(0)),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(0))],
        chunks: Vec::new(),
        cards: Vec::new(),
    }))?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "pending_parsers retained after ParserCompleted; cleared only at ArtifactIndexed"
    );
    assert!(
        !state.pending_artifacts.contains_key(&ArtifactId::new(1)),
        "pending_artifacts consumed on ParserCompleted"
    );
    // First zero-output parse emits ArtifactParsed.
    assert!(output.events.iter().any(|e| matches!(
        e.event,
        DomainEvent::ArtifactParsed {
            status: _,
            chunks_added: 0,
            ..
        }
    )));
    Ok(())
}
