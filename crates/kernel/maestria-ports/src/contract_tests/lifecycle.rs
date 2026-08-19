//! Shared [`EffectJournal`] contract (Rule 25: every concrete port adapter
//! executes the shared contract suite plus adapter-specific boundary tests).

use super::*;
use maestria_domain::{HarnessRunId, ScopeId};

fn intent(
    run_id: u64,
    generation: Option<maestria_domain::JournalGeneration>,
) -> EffectJournalIntent {
    EffectJournalIntent {
        run_id: HarnessRunId::new(run_id),
        task_id: None,
        capability: "shell".to_string(),
        command: "true".to_string(),
        scope_id: ScopeId::new(1),
        requested_generation: generation,
    }
}

fn outcome(run_id: u64) -> HarnessOutcome {
    HarnessOutcome {
        run_id: HarnessRunId::new(run_id),
        command: "echo recovered".to_string(),
        exit_code: 0,
        stdout: b"recovered".to_vec(),
        stderr: Vec::new(),
        duration: std::time::Duration::from_millis(3),
        artifacts_created: Vec::new(),
        diff_summary: None,
        validation_hints: Vec::new(),
    }
}

/// Full lifecycle: intent allocates generation 1, start moves the entry to
/// Started, terminalization removes it from the in-flight scan and makes the
/// generation non-current.
pub fn assert_effect_journal_lifecycle(
    journal: &dyn EffectJournal,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry = journal.record_intent(intent(1, None))?;
    assert_eq!(
        entry.generation.value(),
        1,
        "first intent must allocate generation 1"
    );
    assert_eq!(entry.status, EffectJournalStatus::Intent);

    let in_flight = journal.scan_in_flight()?;
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].status, EffectJournalStatus::Intent);

    journal.record_started(entry.run_id, entry.generation)?;
    let in_flight = journal.scan_in_flight()?;
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].status, EffectJournalStatus::Started);

    assert!(journal.is_current(entry.run_id, entry.generation)?);
    assert!(
        !journal.is_current(
            entry.run_id,
            maestria_domain::JournalGeneration::new(entry.generation.value().saturating_add(1))
        )?,
        "a different generation must not be current"
    );

    journal.record_terminal(
        entry.run_id,
        entry.generation,
        EffectJournalStatus::Completed,
    )?;
    assert!(
        journal.scan_in_flight()?.is_empty(),
        "terminalization must remove the entry from the in-flight scan"
    );
    assert!(
        !journal.is_current(entry.run_id, entry.generation)?,
        "terminalized generation must not be current"
    );
    Ok(())
}

/// Superseding: a second intent for the same run allocates the next
/// generation and marks every earlier in-flight generation non-current.
pub fn assert_effect_journal_supersedes(
    journal: &dyn EffectJournal,
) -> Result<(), Box<dyn std::error::Error>> {
    let first = journal.record_intent(intent(2, None))?;
    assert_eq!(first.generation.value(), 1);
    let second = journal.record_intent(intent(2, None))?;
    assert_eq!(
        second.generation.value(),
        first.generation.value().saturating_add(1),
        "superseding intent must allocate the next generation"
    );

    assert!(!journal.is_current(first.run_id, first.generation)?);
    assert!(journal.is_current(second.run_id, second.generation)?);
    let in_flight = journal.scan_in_flight()?;
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].generation, second.generation);
    Ok(())
}

/// Precondition enforcement: transitions on unknown (run, generation) pairs
/// fail with [`PortError::NotFound`].
pub fn assert_effect_journal_preconditions(
    journal: &dyn EffectJournal,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(matches!(
        journal.record_started(
            HarnessRunId::new(99),
            maestria_domain::JournalGeneration::new(1)
        ),
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        journal.record_terminal(
            HarnessRunId::new(42),
            maestria_domain::JournalGeneration::new(1),
            EffectJournalStatus::Completed
        ),
        Err(PortError::NotFound)
    ));
    Ok(())
}

/// Terminalization semantics: repeating the same status is idempotent, a
/// conflicting status fails, and terminalized entries vanish from
/// `scan_in_flight` without losing accepted feedback.
pub fn assert_effect_journal_terminal_policy(
    journal: &dyn EffectJournal,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry = journal.record_intent(intent(3, None))?;
    journal.record_terminal(entry.run_id, entry.generation, EffectJournalStatus::Failed)?;
    journal.record_terminal(entry.run_id, entry.generation, EffectJournalStatus::Failed)?;
    assert!(matches!(
        journal.record_terminal(
            entry.run_id,
            entry.generation,
            EffectJournalStatus::Completed
        ),
        Err(PortError::NotFound)
    ));
    Ok(())
}

/// Feedback: claiming feedback with an outcome accepts it and retains the
/// outcome for recovery even after terminalization; claiming without an
/// outcome marks the entry as feedback-accepted.
pub fn assert_effect_journal_feedback(
    journal: &dyn EffectJournal,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = HarnessRunId::new(7);
    let entry = journal.record_intent(intent(7, None))?;
    let expected = outcome(7);
    journal.claim_feedback_with_outcome(run_id, entry.generation, expected.clone())?;
    assert!(journal.is_feedback_accepted(run_id, entry.generation)?);
    journal.record_terminal(run_id, entry.generation, EffectJournalStatus::Completed)?;
    assert_eq!(
        journal.feedback_outcome(run_id, entry.generation)?,
        Some(expected),
        "accepted feedback must survive terminalization for recovery"
    );

    let second = journal.record_intent(intent(8, None))?;
    journal.claim_feedback(second.run_id, second.generation)?;
    assert!(journal.is_feedback_accepted(second.run_id, second.generation)?);
    assert!(journal.is_current(second.run_id, second.generation)?);

    let superseding = journal.record_intent(intent(8, None))?;
    assert!(
        !journal.is_current(second.run_id, second.generation)?,
        "feedback-accepted entries must be superseded like in-flight ones"
    );
    assert_eq!(
        superseding.generation.value(),
        second.generation.value().saturating_add(1)
    );
    Ok(())
}

/// The complete shared [`EffectJournal`] suite.
pub fn assert_effect_journal_contract(
    journal: &dyn EffectJournal,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_effect_journal_lifecycle(journal)?;
    assert_effect_journal_supersedes(journal)?;
    assert_effect_journal_preconditions(journal)?;
    assert_effect_journal_terminal_policy(journal)?;
    assert_effect_journal_feedback(journal)?;
    Ok(())
}
