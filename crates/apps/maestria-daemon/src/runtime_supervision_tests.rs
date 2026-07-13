use anyhow::Result;
use maestria_domain::{HarnessRunId, KernelState, ScopeId};
use maestria_ports::{
    EffectJournal, EffectJournalIntent, EffectJournalStatus, InMemoryEffectJournal,
};

use crate::supervise_recovery;

#[test]
fn supervise_recovery_pauses_in_flight_effects() -> Result<()> {
    let state = KernelState::new();
    let journal = InMemoryEffectJournal::default();

    let run_id_1 = HarnessRunId::new(1);
    let run_id_2 = HarnessRunId::new(2);

    // Effect 1 is Intent
    journal.record_intent(EffectJournalIntent {
        run_id: run_id_1,
        task_id: None,
        capability: "shell".to_string(),
        command: "sleep 10".to_string(),
        scope_id: ScopeId::new(1),
        requested_generation: None,
    })?;

    // Effect 2 is Started
    let entry_2 = journal.record_intent(EffectJournalIntent {
        run_id: run_id_2,
        task_id: None,
        capability: "shell".to_string(),
        command: "ls".to_string(),
        scope_id: ScopeId::new(1),
        requested_generation: None,
    })?;
    journal.record_started(run_id_2, entry_2.generation)?;

    // Both are in flight
    let in_flight = journal.scan_in_flight()?;
    assert_eq!(in_flight.len(), 2);

    let diagnostics = supervise_recovery(&state, &journal)?;

    // Must return both as paused
    assert_eq!(diagnostics.paused_effects.len(), 2);
    assert!(
        diagnostics
            .paused_effects
            .iter()
            .all(|e| e.status == EffectJournalStatus::Paused)
    );

    // Must have recorded them as paused in the journal
    let remaining_in_flight = journal.scan_in_flight()?;
    assert!(remaining_in_flight.is_empty(), "all effects must be paused");

    Ok(())
}
