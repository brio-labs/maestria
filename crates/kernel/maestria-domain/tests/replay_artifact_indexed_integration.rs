use maestria_domain::*;

fn new_envelope(id: u64, event: DomainEvent) -> DomainEventEnvelope {
    DomainEventEnvelope {
        id: EventId::new(id),
        sequence: SequenceNumber::new(id),
        event,
    }
}

struct ReplayArtifactSetup<'a> {
    art_id: ArtifactId,
    title: &'a str,
    source_path: &'a str,
    content_hash: &'a str,
    blob_id: BlobId,
    chunk_id: ChunkId,
    chunk_text: &'a str,
}

/// Applies the initial 4-event setup: ArtifactRegistered(1), ParserStarted(2),
/// PendingIndex(3), ChunkRegistered(4). Asserts pending_parsers after ParserStarted.
fn replay_setup_artifact(
    state: &mut KernelState,
    setup: ReplayArtifactSetup<'_>,
) -> Result<(), DomainError> {
    let ReplayArtifactSetup {
        art_id,
        title,
        source_path,
        content_hash,
        blob_id,
        chunk_id,
        chunk_text,
    } = setup;
    state.apply_event(new_envelope(
        1,
        DomainEvent::ArtifactRegistered {
            artifact_id: art_id,
            title: title.to_string(),
        },
    ))?;
    state.apply_event(new_envelope(
        2,
        DomainEvent::ParserStarted {
            artifact_id: art_id,
            title: title.to_string(),
            source_path: source_path.to_string(),
            content_hash: content_hash.to_string(),
            blob_id,
        },
    ))?;
    assert!(state.pending_parsers.contains_key(&art_id));
    state.apply_event(new_envelope(
        3,
        DomainEvent::PendingIndex {
            artifact_id: art_id,
            content_hash: content_hash.to_string(),
        },
    ))?;
    state.apply_event(new_envelope(
        4,
        DomainEvent::ChunkRegistered {
            chunk_id,
            artifact_id: art_id,
            order: 0,
            text: chunk_text.to_string(),
        },
    ))?;
    Ok(())
}

/// Asserts the standard cleanup after an invalid ArtifactIndexed:
/// artifact stays Pending, pending_parsers retained, evidence removed
/// from both maps, and the event preserved in the log.
fn assert_invalid_artifact_indexed_cleanup(
    state: &KernelState,
    art_id: ArtifactId,
    ev_id: EvidenceId,
) {
    assert_eq!(
        state.artifacts[&art_id].index_status,
        IndexStatus::Pending,
        "invalid ArtifactIndexed must leave artifact Pending"
    );
    assert!(
        state.pending_parsers.contains_key(&art_id),
        "pending_parsers must be retained after invalid ArtifactIndexed"
    );
    assert!(
        !state.evidences.contains_key(&ev_id),
        "invalid evidence must be removed from KernelState.evidences"
    );
    assert!(
        !state.artifacts[&art_id].evidence_ids.contains(&ev_id),
        "invalid evidence must be removed from artifact evidence-ID set"
    );
    assert!(
        state.event_log.iter().any(|e| matches!(
            e.event,
            DomainEvent::ArtifactIndexed {
                artifact_id: id,
            } if id == art_id
        )),
        "invalid ArtifactIndexed must be preserved in event log"
    );
}

/// Asserts that the given evidence is tracked in both the global map
/// and the artifact's evidence-ID set.
fn assert_evidence_tracked(state: &KernelState, art_id: ArtifactId, ev_id: EvidenceId, msg: &str) {
    assert!(state.evidences.contains_key(&ev_id), "{}", msg);
    assert!(
        state.artifacts[&art_id].evidence_ids.contains(&ev_id),
        "{}",
        msg
    );
}

/// Asserts terminal state after valid ArtifactIndexed.
fn assert_terminalized(state: &KernelState, art_id: ArtifactId) {
    assert_eq!(state.artifacts[&art_id].index_status, IndexStatus::Indexed,);
    assert!(!state.pending_parsers.contains_key(&art_id));
}

#[test]
fn replay_artifact_indexed_clears_pending_parsers() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Full chain: ArtifactRegistered → ParserStarted → PendingIndex
    // → ChunkRegistered → EvidenceRecorded → FullTextIndexed
    // → ArtifactParsed → ArtifactIndexed.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
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
    assert!(state.pending_parsers.contains_key(&ArtifactId::new(1)));

    // PendingIndex is required to set content_hash on the artifact
    // so the evidence-completeness gate can match hashes.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(1),
            content_hash: "sha256:abc".to_string(),
        },
    })?;

    // Register a chunk so the FullTextIndexed → ArtifactIndexed chain works.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "hello".to_string(),
        },
    })?;

    // Record source-backed FileSpan evidence with snapshot and
    // matching content_hash so the evidence-completeness gate passes.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::EvidenceRecorded {
            evidence_id: evidence_id_for(ArtifactId::new(1), 0),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/notes.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:abc".to_string(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
        },
    })?;

    // FullTextIndexed must be replayed so pending_full_text is empty
    // when ArtifactIndexed arrives.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(6),
        sequence: SequenceNumber::new(6),
        event: DomainEvent::FullTextIndexed {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    })?;

    // ArtifactParsed must NOT clear pending_parsers.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(7),
        sequence: SequenceNumber::new(7),
        event: DomainEvent::ArtifactParsed {
            artifact_id: ArtifactId::new(1),
            chunks_added: 1,
        },
    })?;
    assert!(
        state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactParsed must not clear pending_parsers"
    );

    // ArtifactIndexed (terminal) MUST clear pending_parsers when
    // evidence is complete.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(8),
        sequence: SequenceNumber::new(8),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(1),
        },
    })?;
    assert!(
        !state.pending_parsers.contains_key(&ArtifactId::new(1)),
        "ArtifactIndexed must clear pending_parsers on replay"
    );
    Ok(())
}

#[test]
fn replay_artifact_indexed_rejects_incomplete_evidence() -> Result<(), DomainError> {
    // Regression: when ArtifactIndexed is replayed but no evidence
    // (or non-source-backed evidence) has been recorded, the event
    // must be appended to the event log for replay identity, but
    // the artifact stays Pending and pending_parsers is retained
    // so recovery can retry.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: art_id,
            title: "Notes".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ParserStarted {
            artifact_id: art_id,
            title: "Notes".to_string(),
            source_path: "/tmp/notes.md".to_string(),
            content_hash: "sha256:abc".to_string(),
            blob_id: BlobId::new(42),
        },
    })?;
    assert!(state.pending_parsers.contains_key(&art_id));

    // No PendingIndex, no EvidenceRecorded → evidence_complete_for is false.
    // Set content_hash via PendingIndex so the artifact exists but lacks evidence.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::PendingIndex {
            artifact_id: art_id,
            content_hash: "sha256:abc".to_string(),
        },
    })?;

    // ChunkRegistered so pending_chunks check passes.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(10),
            artifact_id: art_id,
            order: 0,
            text: "hello".to_string(),
        },
    })?;

    // FullTextIndexed so pending_full_text is clear.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::FullTextIndexed {
            artifact_id: art_id,
            chunk_id: ChunkId::new(10),
        },
    })?;

    // ArtifactIndexed with NO evidence — side effects skipped, but
    // the event is preserved in the event log for replay identity.
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(6),
        sequence: SequenceNumber::new(6),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: art_id,
        },
    })?;

    // Artifact MUST stay Pending — evidence missing.
    assert_eq!(
        state.artifacts[&art_id].index_status,
        IndexStatus::Pending,
        "replay ArtifactIndexed without evidence must leave artifact Pending"
    );
    assert!(
        state.pending_parsers.contains_key(&art_id),
        "pending_parsers must be retained when replay ArtifactIndexed has incomplete evidence"
    );
    // The event MUST be in the event log even though side effects were skipped.
    assert!(
        state.event_log.iter().any(|e| matches!(
            e.event,
            DomainEvent::ArtifactIndexed {
                artifact_id: ArtifactId(1)
            }
        )),
        "ArtifactIndexed event must be preserved in event log for replay identity"
    );
    assert_eq!(
        state.event_log.len(),
        6,
        "all 6 replayed events must be in the log"
    );
    Ok(())
}

#[test]
fn replay_artifact_indexed_removes_invalid_evidence() -> Result<(), DomainError> {
    // Regression: when an invalid ArtifactIndexed is replayed with
    // source-evidence records that fail validation (wrong hash, missing
    // snapshot, artifact mismatch), those records MUST be removed from
    // KernelState.evidences and artifact.evidence_ids so that a
    // subsequent valid RecordEvidence for the same deterministic ID
    // is accepted instead of rejected as a duplicate.
    let mut state = KernelState::new();
    let art_id = ArtifactId::new(1);
    let det_ev_id = evidence_id_for(art_id, 0);

    replay_setup_artifact(
        &mut state,
        ReplayArtifactSetup {
            art_id,
            title: "Notes",
            source_path: "/tmp/notes.md",
            content_hash: "sha256:abc",
            blob_id: BlobId::new(42),
            chunk_id: ChunkId::new(10),
            chunk_text: "hello",
        },
    )?;

    // Record deterministic evidence with a WRONG content_hash so
    // evidence_complete_for returns false but the record exists.
    state.apply_event(new_envelope(
        5,
        DomainEvent::EvidenceRecorded {
            evidence_id: det_ev_id,
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/notes.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:wrong".to_string(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
        },
    ))?;

    // Confirm invalid evidence landed in state.
    assert!(
        state.evidences.contains_key(&det_ev_id),
        "invalid evidence must exist before ArtifactIndexed"
    );
    assert!(
        state.artifacts[&art_id].evidence_ids.contains(&det_ev_id),
        "artifact must track the invalid evidence id"
    );

    state.apply_event(new_envelope(
        6,
        DomainEvent::FullTextIndexed {
            artifact_id: art_id,
            chunk_id: ChunkId::new(10),
        },
    ))?;

    state.apply_event(new_envelope(
        7,
        DomainEvent::ArtifactParsed {
            artifact_id: art_id,
            chunks_added: 1,
        },
    ))?;

    // ArtifactIndexed with invalid evidence — must NOT terminalize.
    state.apply_event(new_envelope(
        8,
        DomainEvent::ArtifactIndexed {
            artifact_id: art_id,
        },
    ))?;

    assert_invalid_artifact_indexed_cleanup(&state, art_id, det_ev_id);

    // Now record a VALID EvidenceRecorded for the same deterministic ID.
    // This MUST succeed — the invalid record was removed above.
    state.apply_event(new_envelope(
        9,
        DomainEvent::EvidenceRecorded {
            evidence_id: det_ev_id,
            artifact_id: art_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/notes.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:abc".to_string(),
                snapshot: Some(BlobId::new(42)),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
        },
    ))?;

    assert_evidence_tracked(
        &state,
        art_id,
        det_ev_id,
        "valid evidence must be accepted for the same deterministic ID after removal",
    );

    // Now evidence is complete — ArtifactIndexed should terminalize.
    state.apply_event(new_envelope(
        10,
        DomainEvent::ArtifactIndexed {
            artifact_id: art_id,
        },
    ))?;

    assert_terminalized(&state, art_id);
    Ok(())
}

fn record_cross_owned_evidence(
    state: &mut KernelState,
    artifact_id: ArtifactId,
    evidence_id: EvidenceId,
) -> Result<(), DomainError> {
    state.apply_event(new_envelope(
        6,
        DomainEvent::EvidenceRecorded {
            evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/a.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:aaa".to_string(),
                snapshot: Some(BlobId::new(10)),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
        },
    ))?;
    Ok(())
}

fn assert_cross_owned_cleanup(
    state: &mut KernelState,
    art_a: ArtifactId,
    art_b: ArtifactId,
    det_ev_id: EvidenceId,
) -> Result<(), DomainError> {
    assert!(state.artifacts[&art_b].evidence_ids.contains(&det_ev_id));
    assert!(state.evidences.contains_key(&det_ev_id));
    state.apply_event(new_envelope(
        7,
        DomainEvent::FullTextIndexed {
            artifact_id: art_a,
            chunk_id: ChunkId::new(10),
        },
    ))?;
    state.apply_event(new_envelope(
        8,
        DomainEvent::ArtifactParsed {
            artifact_id: art_a,
            chunks_added: 1,
        },
    ))?;
    state.apply_event(new_envelope(
        9,
        DomainEvent::ArtifactIndexed { artifact_id: art_a },
    ))?;
    assert!(!state.evidences.contains_key(&det_ev_id));
    assert!(!state.artifacts[&art_b].evidence_ids.contains(&det_ev_id));
    assert!(!state.artifacts[&art_a].evidence_ids.contains(&det_ev_id));
    assert_eq!(state.artifacts[&art_a].index_status, IndexStatus::Pending);
    assert_eq!(state.event_log.len(), 9);
    assert!(state.event_log.iter().any(|e| matches!(
        e.event,
        DomainEvent::ArtifactIndexed { artifact_id: id } if id == art_a
    )));
    Ok(())
}

#[test]
fn replay_artifact_indexed_cleans_cross_artifact_evidence_owner() -> Result<(), DomainError> {
    // Regression: ArtifactIndexed cleanup must remove cross-owned evidence IDs
    // from the actual owner artifact's evidence_ids, not just the indexed target.
    let mut state = KernelState::new();
    let art_a = ArtifactId::new(1);
    let art_b = ArtifactId::new(2);
    let det_ev_id = evidence_id_for(art_a, 0);

    // Register artifact A (the indexed target).
    replay_setup_artifact(
        &mut state,
        ReplayArtifactSetup {
            art_id: art_a,
            title: "NotesA",
            source_path: "/tmp/a.md",
            content_hash: "sha256:aaa",
            blob_id: BlobId::new(10),
            chunk_id: ChunkId::new(10),
            chunk_text: "hello",
        },
    )?;

    // Register artifact B (the cross-owner sink).
    state.apply_event(new_envelope(
        5,
        DomainEvent::ArtifactRegistered {
            artifact_id: art_b,
            title: "NotesB".to_string(),
        },
    ))?;

    // Record deterministic evidence for A's chunk under B's ownership.
    record_cross_owned_evidence(&mut state, art_b, det_ev_id)?;

    assert_cross_owned_cleanup(&mut state, art_a, art_b, det_ev_id)?;

    // ---- Record valid evidence for the same deterministic ID ----
    state.apply_event(new_envelope(
        10,
        DomainEvent::EvidenceRecorded {
            evidence_id: det_ev_id,
            artifact_id: art_a, // correct owner this time
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/a.md".to_string(),
                range: ContentRange { start: 0, end: 1 },
                content_hash: "sha256:aaa".to_string(),
                snapshot: Some(BlobId::new(10)),
            },
            excerpt: "hello".to_string(),
            observed_at: LogicalTick::new(1),
        },
    ))?;

    assert_evidence_tracked(
        &state,
        art_a,
        det_ev_id,
        "valid replacement evidence must be accepted",
    );

    // ---- Terminal ArtifactIndexed ----
    state.apply_event(new_envelope(
        11,
        DomainEvent::ArtifactIndexed { artifact_id: art_a },
    ))?;

    assert_eq!(
        state.artifacts[&art_a].index_status,
        IndexStatus::Indexed,
        "artifact A must terminalize after valid evidence is recorded"
    );

    Ok(())
}
