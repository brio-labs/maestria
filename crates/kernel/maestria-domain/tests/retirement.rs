use maestria_domain::*;

// ── Retrieval audit retirement (ADR-0009) ─────────────────────────

fn retire(before_sequence: u64, reason: &str) -> DomainInput {
    DomainInput::RetrievalEventsRetired(RetrievalEventsRetired {
        before_sequence,
        reason: reason.to_string(),
    })
}

fn last_marker(state: &KernelState) -> Result<(u64, String), DomainError> {
    let envelope = state.event_log_owned().into_iter().next_back().ok_or(
        DomainError::InternalInvariantViolation {
            detail: "expected a retirement marker event",
        },
    )?;
    match envelope.event {
        DomainEvent::RetrievalEventsRetired {
            before_sequence,
            reason,
        } => Ok((before_sequence, reason)),
        _ => Err(DomainError::InternalInvariantViolation {
            detail: "expected a retirement marker event",
        }),
    }
}

#[test]
fn retirement_records_marker_and_advances_high_water() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(retire(40, "quarterly audit policy"))?;

    assert_eq!(state.retrieval_retired_through, 40);
    assert_eq!(last_marker(&state)?, (40, "quarterly audit policy".into()));
    Ok(())
}

#[test]
fn retirement_rejects_empty_reason_atomically() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let result = state.apply_input(retire(40, "   "));

    assert!(matches!(result, Err(DomainError::EmptyRetirementReason)));
    assert_eq!(state.retrieval_retired_through, 0);
    assert!(state.event_log_owned().is_empty());
    Ok(())
}

#[test]
fn retirement_below_high_water_is_recorded_noop() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(retire(100, "initial sweep"))?;
    state.apply_input(retire(40, "late lower request"))?;

    assert_eq!(state.retrieval_retired_through, 100);
    // Both markers are recorded; the late lower request changed nothing.
    assert_eq!(state.event_log_owned().len(), 2);
    assert_eq!(last_marker(&state)?, (40, "late lower request".into()));
    Ok(())
}

#[test]
fn replay_restores_retirement_high_water() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(retire(100, "initial sweep"))?;
    state.apply_input(retire(40, "late lower request"))?;
    let envelopes = state.event_log_owned();

    let replayed = replay_events(envelopes)?;
    assert_eq!(replayed.retrieval_retired_through, 100);
    Ok(())
}
