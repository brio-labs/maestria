use std::sync::{Mutex, MutexGuard};

use maestria_domain::HarnessRunId;

use crate::{
    EffectJournal, EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, PortError,
};

#[derive(Debug, Default)]
pub struct InMemoryEffectJournal {
    entries: Mutex<Vec<EffectJournalEntry>>,
}

impl InMemoryEffectJournal {
    fn lock(&self) -> Result<MutexGuard<'_, Vec<EffectJournalEntry>>, PortError> {
        self.entries.lock().map_err(|_| PortError::Internal {
            message: "effect journal lock poisoned".to_string(),
        })
    }
}

impl EffectJournal for InMemoryEffectJournal {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        let mut entries = self.lock()?;
        let previous_generation = entries
            .iter()
            .filter(|entry| entry.run_id == intent.run_id)
            .map(|entry| entry.generation)
            .max();
        let next_generation = previous_generation.map_or(1, |value| value.saturating_add(1));
        let generation = match intent.requested_generation {
            Some(requested) if requested >= next_generation => requested,
            _ => next_generation,
        };
        for entry in entries.iter_mut().filter(|entry| {
            entry.run_id == intent.run_id
                && matches!(
                    entry.status,
                    EffectJournalStatus::Intent | EffectJournalStatus::Started
                )
        }) {
            entry.status = EffectJournalStatus::Superseded;
        }
        let entry = EffectJournalEntry {
            run_id: intent.run_id,
            task_id: intent.task_id,
            capability: intent.capability,
            command: intent.command,
            scope_id: intent.scope_id,
            generation,
            status: EffectJournalStatus::Intent,
        };
        entries.push(entry.clone());
        Ok(entry)
    }

    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        let mut entries = self.lock()?;
        let entry = entries
            .iter_mut()
            .find(|entry| {
                entry.run_id == run_id
                    && entry.generation == generation
                    && entry.status == EffectJournalStatus::Intent
            })
            .ok_or(PortError::NotFound)?;
        entry.status = EffectJournalStatus::Started;
        Ok(())
    }

    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        if !matches!(
            status,
            EffectJournalStatus::Completed
                | EffectJournalStatus::Failed
                | EffectJournalStatus::Paused
                | EffectJournalStatus::Superseded
        ) {
            return Err(PortError::InvalidInput {
                message: "terminal journal status required".to_string(),
            });
        }
        let mut entries = self.lock()?;
        let entry = entries
            .iter_mut()
            .find(|entry| {
                entry.run_id == run_id
                    && entry.generation == generation
                    && matches!(
                        entry.status,
                        EffectJournalStatus::Intent | EffectJournalStatus::Started
                    )
            })
            .ok_or(PortError::NotFound)?;
        entry.status = status;
        Ok(())
    }

    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        Ok(self
            .lock()?
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    EffectJournalStatus::Intent | EffectJournalStatus::Started
                )
            })
            .cloned()
            .collect())
    }

    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        Ok(self
            .lock()?
            .iter()
            .rev()
            .find(|entry| entry.run_id == run_id && entry.generation == generation)
            .is_some_and(|entry| {
                matches!(
                    entry.status,
                    EffectJournalStatus::Intent | EffectJournalStatus::Started
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::{HarnessRunId, ScopeId};

    use super::*;

    fn intent(run_id: u64, generation: Option<u64>) -> EffectJournalIntent {
        EffectJournalIntent {
            run_id: HarnessRunId::new(run_id),
            task_id: None,
            capability: "shell".to_string(),
            command: "true".to_string(),
            scope_id: ScopeId::new(1),
            requested_generation: generation,
        }
    }

    #[test]
    fn records_lifecycle_and_current_generation() -> Result<(), PortError> {
        let journal = InMemoryEffectJournal::default();
        let entry = journal.record_intent(intent(1, None))?;
        assert_eq!(entry.generation, 1);
        journal.record_started(entry.run_id, entry.generation)?;
        assert!(journal.is_current(entry.run_id, entry.generation)?);
        journal.record_terminal(
            entry.run_id,
            entry.generation,
            EffectJournalStatus::Completed,
        )?;
        assert!(!journal.is_current(entry.run_id, entry.generation)?);
        Ok(())
    }

    #[test]
    fn superseding_marks_old_generation_and_increments() -> Result<(), PortError> {
        let journal = InMemoryEffectJournal::default();
        let first = journal.record_intent(intent(1, None))?;
        let second = journal.record_intent(intent(1, None))?;
        assert_eq!(second.generation, first.generation + 1);
        assert!(!journal.is_current(first.run_id, first.generation)?);
        assert!(journal.is_current(second.run_id, second.generation)?);
        Ok(())
    }

    #[test]
    fn scans_only_unfinished_entries() -> Result<(), PortError> {
        let journal = InMemoryEffectJournal::default();
        let entry = journal.record_intent(intent(1, None))?;
        assert_eq!(journal.scan_in_flight()?.len(), 1);
        journal.record_terminal(entry.run_id, entry.generation, EffectJournalStatus::Paused)?;
        assert!(journal.scan_in_flight()?.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_non_terminal_status() -> Result<(), PortError> {
        let journal = InMemoryEffectJournal::default();
        let entry = journal.record_intent(intent(1, None))?;
        let result =
            journal.record_terminal(entry.run_id, entry.generation, EffectJournalStatus::Started);
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
        Ok(())
    }
}
