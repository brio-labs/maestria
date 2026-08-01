use maestria_ports::{
    EffectJournal, EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, HarnessOutcome,
    HarnessRunId, PortError,
};

use crate::{SqliteStore, repositories};

impl EffectJournal for SqliteStore {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError> {
        let mut connection = self.lock()?;
        repositories::effect_journal_repo::record_intent(&mut connection, intent)
    }

    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::record_started(&connection, run_id, generation)
    }
    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::claim_feedback(&connection, run_id, generation)
    }

    fn claim_feedback_with_outcome(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        outcome: HarnessOutcome,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::claim_feedback_with_outcome(
            &connection,
            run_id,
            generation,
            Some(&outcome),
        )
    }

    fn feedback_outcome(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<Option<HarnessOutcome>, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::feedback_outcome(&connection, run_id, generation)
    }

    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::record_terminal(&connection, run_id, generation, status)
    }

    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::scan_in_flight(&connection)
    }

    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::is_feedback_accepted(&connection, run_id, generation)
    }
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError> {
        let connection = self.lock()?;
        repositories::effect_journal_repo::is_current(&connection, run_id, generation)
    }
}
