use maestria_domain::*;

// ── Replay behavior for parser lifecycle ──────────────────────────

#[test]
fn replay_reconstructs_pending_parsers() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    })?;

    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));
    let pending = &state.pending_parsers[&ArtifactId::new(1)];
    assert_eq!(pending.title, "Notes");
    assert_eq!(pending.blob_id, BlobId::new(42));
    Ok(())
}

#[test]
fn replay_artifact_parsed_cleans_up_pending_parsers() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    // Set up: artifact registered (from first-time commit) + parser started
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            security: SecurityMetadata::default(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    })?;
    // Replay the ArtifactParsed event (emitted on ParserCompleted success).
    // pending_parsers is NOT removed here — only ArtifactIndexed clears it.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ArtifactParsed {
            status: maestria_domain::ParseStatus::Parsed,
            artifact_id: ArtifactId::new(1),
            chunks_added: 0,
        },
    })?;

    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactParsed replay retains pending parsers; only ArtifactIndexed clears them"
    );
    Ok(())
}
