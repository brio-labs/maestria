use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/deterministic.rs"]
mod deterministic_helpers;

use assertions::require_error;
use deterministic_helpers::{
    malformed_deterministic_evidence_events, valid_duplicate_evidence_events,
};

// ── Deterministic evidence validation on replay ───────────────────

#[test]
fn replay_events_rejects_malformed_deterministic_evidence_before_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let evidence_id = evidence_id_for(art_id, 0);
    let events = malformed_deterministic_evidence_events(art_id, chunk_id, evidence_id)?;
    let error = require_error(
        replay_events(&events),
        "malformed deterministic evidence must fail replay",
    )?;
    assert!(matches!(
        error,
        DomainError::MalformedDeterministicEvidence {
            evidence_id: rejected_id,
            ..
        } if rejected_id == evidence_id
    ));
    Ok(())
}

#[test]
fn replay_rejects_evidence_before_chunk_without_mutating_chunk_state()
-> Result<(), Box<dyn std::error::Error>> {
    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let evidence_id = evidence_id_for(art_id, 0);
    let mut events = malformed_deterministic_evidence_events(art_id, chunk_id, evidence_id)?;
    events.swap(4, 5);
    events[4].id = EventId::new(5);
    events[5].id = EventId::new(6);

    let mut state = KernelState::new();
    for event in events.iter().take(5) {
        state.apply_event(event.clone())?;
    }
    let before = state.clone();
    let error = require_error(
        state.apply_event(events[5].clone()),
        "evidence preceding its deterministic chunk must fail at chunk registration",
    )?;
    assert!(matches!(
        error,
        DomainError::MalformedDeterministicEvidence {
            evidence_id: rejected_id,
            ..
        } if rejected_id == evidence_id
    ));
    assert_eq!(state, before, "failed replay chunk registration is atomic");
    Ok(())
}

#[test]
fn replay_events_valid_duplicate_evidence_still_errors() -> Result<(), Box<dyn std::error::Error>> {
    // A valid deterministic evidence record followed by a *different*
    // valid record at the same ID must still fail replay.
    let events = valid_duplicate_evidence_events()?;
    let err = require_error(replay_events(&events), "duplicate valid evidence must fail")?;
    assert!(
        matches!(err, DomainError::DuplicateEvidence { .. }),
        "expected DuplicateEvidence error, got {:?}",
        err
    );
    Ok(())
}
