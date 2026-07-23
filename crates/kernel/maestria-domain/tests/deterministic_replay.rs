use maestria_domain::*;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/deterministic.rs"]
mod deterministic_helpers;

use assertions::require_error;
use deterministic_helpers::{
    malformed_to_valid_replacement_events, valid_duplicate_evidence_events,
};

// ── Deterministic evidence validation on replay ───────────────────

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
