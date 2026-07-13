use anyhow::{Context, Result};
use maestria_domain::KernelState;
use maestria_ports::{EffectJournal, EffectJournalEntry, EffectJournalStatus};

use crate::{RecoveryInputs, recovery_inputs};

/// Public diagnostics computed during startup before any runtime replay.
/// This exposes paused in-flight non-idempotent work without replaying it.
#[derive(Debug, Clone)]
pub struct RecoveryDiagnostics {
    pub inputs: RecoveryInputs,
    pub paused_effects: Vec<EffectJournalEntry>,
}

/// Pause non-idempotent harness effects left in flight by a previous process.
///
/// Effects are not automatically replayed: an operator must explicitly approve
/// any future retry, preventing a crash from duplicating an external action.
pub fn supervise_recovery(
    state: &KernelState,
    journal: &dyn EffectJournal,
) -> Result<RecoveryDiagnostics> {
    let inputs = recovery_inputs(state);
    let in_flight = journal
        .scan_in_flight()
        .with_context(|| "scan in-flight harness effects")?;

    let mut paused_effects = Vec::new();
    for mut entry in in_flight {
        journal
            .record_terminal(entry.run_id, entry.generation, EffectJournalStatus::Paused)
            .with_context(|| format!("pause in-flight effect {}", entry.run_id))?;
        tracing::info!(
            run_id = %entry.run_id,
            capability = %entry.capability,
            "paused in-flight harness effect on restart"
        );
        entry.status = EffectJournalStatus::Paused;
        paused_effects.push(entry);
    }

    Ok(RecoveryDiagnostics {
        inputs,
        paused_effects,
    })
}
