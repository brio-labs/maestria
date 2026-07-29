use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use crate::effect_result::EffectFailure;
use crate::harness::truncate_output;
use crate::runtime::MaestriaRuntime;
use maestria_domain::{
    DomainInput, KernelState, MaestriaEffect, ModelAgentHarnessResult, ModelAgentProposalExecution,
    ModelAgentProposalRequest, ScopeId,
};
use maestria_ports::EffectJournalEntry;
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::mpsc;

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

impl MaestriaRuntime {
    pub(crate) fn plan_model_agent_recovery(
        &self,
        snapshot: &KernelState,
    ) -> Vec<ModelAgentProposalRequest> {
        let mut proposals = BTreeMap::new();
        let mut approval_owned_runs = BTreeSet::new();
        match self.adapters.effect_journal.scan_in_flight() {
            Ok(entries) => {
                for entry in entries {
                    if entry.status != maestria_ports::EffectJournalStatus::FeedbackAccepted
                        || entry.feedback.is_none()
                        || snapshot.model_agent_results.contains_key(&entry.run_id)
                    {
                        continue;
                    }
                    let Some(proposal) = snapshot.model_agent_requests.get(&entry.run_id) else {
                        continue;
                    };
                    if !matches!(&proposal.execution, ModelAgentProposalExecution::Fresh)
                        || !journal_entry_matches_proposal(&entry, proposal, self.config.scope_id)
                    {
                        continue;
                    }
                    let mut resumed = proposal.clone();
                    resumed.execution = ModelAgentProposalExecution::JournalRecovery {
                        journal_generation: entry.generation,
                    };
                    proposals.insert(entry.run_id, resumed);
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to scan effect journal for model-agent recovery");
            }
        }
        match self.adapters.approval_repo.find_all() {
            Ok(records) => {
                for record in records {
                    let Some(proposal) =
                        crate::effect_execution::decode_pending_continuation(&record)
                    else {
                        continue;
                    };
                    if !snapshot.model_agent_requests.contains_key(&proposal.run_id) {
                        continue;
                    }
                    approval_owned_runs.insert(proposal.run_id);
                    if !matches!(
                        record.status,
                        maestria_ports::ApprovalStatus::Approved
                            | maestria_ports::ApprovalStatus::Denied
                    ) || snapshot.model_agent_results.contains_key(&proposal.run_id)
                    {
                        continue;
                    }
                    proposals.entry(proposal.run_id).or_insert(proposal);
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to scan approvals for model-agent recovery");
            }
        }
        for proposal in snapshot.model_agent_requests.values() {
            if matches!(&proposal.execution, ModelAgentProposalExecution::Fresh)
                && !snapshot.model_agent_results.contains_key(&proposal.run_id)
                && !approval_owned_runs.contains(&proposal.run_id)
                && !proposals.contains_key(&proposal.run_id)
            {
                proposals.insert(proposal.run_id, proposal.clone());
            }
        }
        proposals.into_values().collect()
    }

    pub(crate) async fn queue_model_agent_recovery(
        proposals: Vec<ModelAgentProposalRequest>,
        shutdown_token: &tokio_util::sync::CancellationToken,
        effect_tx: &mpsc::Sender<crate::effect_dispatch::EffectBatch>,
    ) {
        for proposal in proposals {
            tokio::select! {
                () = shutdown_token.cancelled() => break,
                result = effect_tx.send(vec![EffectWork::Pending(
                    MaestriaEffect::QueryHarnessProposal(proposal.into_harness_request()),
                )]) => {
                    if let Err(error) = result {
                        tracing::warn!(%error, "model-agent recovery effect channel closed");
                        break;
                    }
                }
            }
        }
    }
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
