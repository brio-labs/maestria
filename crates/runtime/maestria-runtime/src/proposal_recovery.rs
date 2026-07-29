use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use crate::harness::truncate_output;
use maestria_domain::{DomainInput, ModelAgentHarnessResult, ModelAgentProposalRequest, ScopeId};
use maestria_ports::EffectJournalEntry;

pub(crate) fn journal_entry_matches_proposal(
    entry: &EffectJournalEntry,
    proposal: &ModelAgentProposalRequest,
    scope_id: ScopeId,
) -> bool {
    entry.run_id == proposal.run_id
        && entry.task_id == proposal.task_id
        && entry.capability == proposal.capability
        && entry.command == proposal.command
        && entry.scope_id == scope_id
}

impl EffectExecutionContext {
    pub(crate) async fn execute_recovered_harness(
        &self,
        proposal: &ModelAgentProposalRequest,
        generation: u64,
    ) -> Result<Option<ModelAgentHarnessResult>, EffectFailure> {
        let entry = self
            .adapters
            .effect_journal
            .scan_in_flight()
            .map_err(|error| {
                EffectFailure::Failed(format!("scan recovered harness journal: {error}"))
            })?
            .into_iter()
            .find(|entry| {
                entry.generation == generation
                    && journal_entry_matches_proposal(entry, proposal, self.scope_id)
            })
            .ok_or_else(|| {
                EffectFailure::Failed(
                    "journal recovery does not match an exact harness journal entry".to_string(),
                )
            })?;
        if entry.status != maestria_ports::EffectJournalStatus::FeedbackAccepted {
            return Err(EffectFailure::Failed(
                "journal recovery requires accepted harness feedback".to_string(),
            ));
        }
        let outcome = entry.feedback.ok_or_else(|| {
            EffectFailure::Failed("journal recovery feedback is not durable".to_string())
        })?;
        if outcome.run_id != proposal.run_id || outcome.command != proposal.command {
            return Err(EffectFailure::Failed(
                "journal recovery feedback identity does not match its proposal".to_string(),
            ));
        }
        let mut output = String::from_utf8_lossy(&outcome.stdout).into_owned();
        if !outcome.stderr.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&String::from_utf8_lossy(&outcome.stderr));
        }
        Self::send_input(
            &self.input_tx,
            DomainInput::HarnessRunCompleted(maestria_domain::HarnessRunCompleted {
                run_id: proposal.run_id,
                generation,
                task_id: proposal.task_id,
                command: outcome.command.clone(),
                exit_code: outcome.exit_code,
                output,
            }),
            "recovered harness completion",
        )
        .map_err(|error| {
            EffectFailure::Degraded(format!("deliver recovered harness result: {error}"))
        })?;
        Ok(Some(ModelAgentHarnessResult {
            exit_code: outcome.exit_code,
            stdout: truncate_output(&outcome.stdout),
            stderr: truncate_output(&outcome.stderr),
            duration_ms: outcome.duration.as_millis().min(u128::from(u64::MAX)) as u64,
        }))
    }
}
