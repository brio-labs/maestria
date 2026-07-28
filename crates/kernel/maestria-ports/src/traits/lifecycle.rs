use crate::PortError;
use maestria_domain::{HarnessRunId, ScopeId, TaskId};

use super::harness::HarnessOutcome;
/// Lifecycle state for a supervised non-idempotent effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectJournalStatus {
    Intent,
    Started,
    FeedbackAccepted,
    Completed,
    Failed,
    Paused,
    Superseded,
}

/// Runtime-owned request persisted before a harness effect starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectJournalIntent {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub capability: String,
    pub command: String,
    pub scope_id: ScopeId,
    pub requested_generation: Option<u64>,
}

/// Durable lifecycle entry for one supervised effect generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectJournalEntry {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub capability: String,
    pub command: String,
    pub scope_id: ScopeId,
    pub generation: u64,
    pub status: EffectJournalStatus,
    /// Successful harness feedback retained for crash recovery.
    pub feedback: Option<HarnessOutcome>,
}

impl EffectJournalEntry {
    pub fn feedback_outcome(&self) -> Option<&HarnessOutcome> {
        self.feedback.as_ref()
    }
}

/// Durable supervision journal for non-idempotent effect execution.
pub trait EffectJournal: Send + Sync {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError>;
    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError>;
    /// Atomically claims feedback for the current generation before enqueueing it.
    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError>;
    /// Atomically stores successful feedback and marks it accepted.
    ///
    /// The default preserves adapters implemented against the legacy port.
    fn claim_feedback_with_outcome(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        outcome: HarnessOutcome,
    ) -> Result<(), PortError> {
        let _ = outcome;
        self.claim_feedback(run_id, generation)
    }
    fn feedback_outcome(
        &self,
        _run_id: HarnessRunId,
        _generation: u64,
    ) -> Result<Option<HarnessOutcome>, PortError> {
        Ok(None)
    }
    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError>;
    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError>;
    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError>;
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError>;
}
