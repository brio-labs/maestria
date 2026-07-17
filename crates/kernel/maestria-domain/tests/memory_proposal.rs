use maestria_domain::*;
use std::collections::BTreeSet;

#[path = "common/memory.rs"]
mod common;
use common::{file_span_kind, register_artifact_and_claim};

#[test]
fn propose_memory_candidate_creates_claim_and_candidate_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(12),
        security: None,
    }))?;

    let output = state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "The project uses Rust".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 750,
            security: None,
        },
    ))?;

    // Verify state mutations.
    let claim = state
        .claims
        .get(&ClaimId::new(20))
        .ok_or(DomainError::MissingClaim {
            id: ClaimId::new(20),
        })?;
    assert_eq!(claim.text, "The project uses Rust");
    assert_eq!(claim.artifact_id, ArtifactId::new(1));
    assert_eq!(claim.evidence_ids, BTreeSet::from([EvidenceId::new(40)]));

    let candidate = state
        .memory_candidates
        .get(&MemoryCandidateId::new(90))
        .ok_or(DomainError::MissingMemoryCandidate {
            id: MemoryCandidateId::new(90),
        })?;
    assert_eq!(candidate.claim_id, ClaimId::new(20));
    assert_eq!(candidate.confidence_milli, 750);
    assert_eq!(
        candidate.evidence_ids,
        BTreeSet::from([EvidenceId::new(40)])
    );

    // Evidence claim_id was updated.
    let evidence =
        state
            .evidences
            .get(&EvidenceId::new(40))
            .ok_or(DomainError::MissingEvidence {
                id: EvidenceId::new(40),
            })?;
    assert_eq!(evidence.claim_id, Some(ClaimId::new(20)));

    // Verify both events emitted and persisted.
    assert_eq!(output.events.len(), 2);
    assert!(matches!(
        output.events[0].event,
        DomainEvent::ClaimCreated { .. }
    ));
    assert!(matches!(
        output.events[1].event,
        DomainEvent::MemoryCandidateCreated { .. }
    ));
    assert_eq!(output.events[0].id, EventId::new(3));
    assert_eq!(output.events[1].id, EventId::new(4));

    // Both events generate PersistEvent effects.
    assert_eq!(output.effects.len(), 2);
    for (i, effect) in output.effects.iter().enumerate() {
        assert!(matches!(effect, MaestriaEffect::PersistEvent { .. }));
        if let MaestriaEffect::PersistEvent { envelope } = effect {
            assert_eq!(envelope.as_ref(), &output.events[i]);
        }
    }

    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_empty_text() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "   ".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => return Err(std::io::Error::other("empty text must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::EmptyClaimText));
    // Verify no partial state mutation.
    assert!(!state.claims.contains_key(&ClaimId::new(20)));
    assert!(
        !state
            .memory_candidates
            .contains_key(&MemoryCandidateId::new(90))
    );
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_invalid_confidence() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "valid claim".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 1001,
            security: None,
        },
    )) {
        Ok(_) => return Err(std::io::Error::other("confidence > 1000 must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(
        err,
        DomainError::InvalidConfidence {
            max: 1000,
            actual: 1001,
        }
    ));
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_missing_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "valid claim".to_string(),
            evidence_ids: vec![EvidenceId::new(99)],
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => return Err(std::io::Error::other("missing evidence must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(
        err,
        DomainError::MissingEvidence { id } if id == EvidenceId::new(99)
    ));
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_empty_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "valid claim".to_string(),
            evidence_ids: Vec::new(),
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => return Err(std::io::Error::other("empty evidence must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::EvidenceRequired { .. }));
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_duplicate_claim_id() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id: ClaimId::new(20),
        artifact_id: ArtifactId::new(1),
        text: "existing claim".to_string(),
        evidence_ids: Vec::new(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "new proposal".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => return Err(std::io::Error::other("duplicate claim_id must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::DuplicateId { kind, .. } if kind == "claim"));
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_duplicate_candidate_id()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(90),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 500,
            security: None,
        },
    ))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(21),
            candidate_id: MemoryCandidateId::new(90),
            text: "new proposal".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => {
            return Err(std::io::Error::other("duplicate candidate_id must be rejected").into());
        }
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::DuplicateId { kind, .. } if kind == "memory_candidate"));
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_artifact_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(2),
        title: "Other Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(41),
        artifact_id: ArtifactId::new(2),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "valid claim".to_string(),
            evidence_ids: vec![EvidenceId::new(40), EvidenceId::new(41)],
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => return Err(std::io::Error::other("artifact mismatch must be rejected").into()),
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::ArtifactMismatch { .. }));
    Ok(())
}

#[test]
fn propose_memory_candidate_rejects_evidence_bound_to_other_claim()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    let err = match state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(21),
            candidate_id: MemoryCandidateId::new(90),
            text: "new proposal".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 500,
            security: None,
        },
    )) {
        Ok(_) => {
            return Err(
                std::io::Error::other("evidence bound to other claim must be rejected").into(),
            );
        }
        Err(error) => error,
    };
    assert!(matches!(err, DomainError::DuplicateId { kind, .. } if kind == "evidence_claim"));
    Ok(())
}

#[test]
fn propose_memory_candidate_survives_replay() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_domain::replay_events;

    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    let _ = state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "The project uses Rust".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 750,
            security: None,
        },
    ))?;

    // Use the full event log from the state for replay.
    let replayed = replay_events(&state.event_log)?;

    assert!(replayed.claims.contains_key(&ClaimId::new(20)));
    assert!(
        replayed
            .memory_candidates
            .contains_key(&MemoryCandidateId::new(90))
    );
    let claim = replayed
        .claims
        .get(&ClaimId::new(20))
        .ok_or(DomainError::MissingClaim {
            id: ClaimId::new(20),
        })?;
    assert_eq!(claim.text, "The project uses Rust");
    Ok(())
}

#[test]
fn propose_memory_candidate_does_not_promote() -> Result<(), Box<dyn std::error::Error>> {
    // Verify that proposal never creates a Memory entry — promotion
    // remains an explicit separate step.
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: file_span_kind(),
        excerpt: "text".to_string(),
        observed_at: LogicalTick::new(0),
        security: None,
    }))?;

    state.apply_input(DomainInput::ProposeMemoryCandidate(
        ProposeMemoryCandidateInput {
            claim_id: ClaimId::new(20),
            candidate_id: MemoryCandidateId::new(90),
            text: "The project uses Rust".to_string(),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 950,
            security: None,
        },
    ))?;

    // Even with high confidence, no memory entry is created.
    assert!(state.memories.is_empty());

    // But the candidate exists and is eligible for promotion.
    let candidate = state
        .memory_candidates
        .get(&MemoryCandidateId::new(90))
        .ok_or(DomainError::MissingMemoryCandidate {
            id: MemoryCandidateId::new(90),
        })?;
    assert!(candidate.confidence_milli >= 500);

    Ok(())
}
