use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use crate::effect_result::EffectFailure;
use crate::harness::model_agent_harness_result;
use crate::runtime::MaestriaRuntime;
use maestria_domain::{
    KernelState, MaestriaEffect, ModelAgentHarnessResult, ModelAgentProposalExecution,
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
    /// Plan model-agent recovery from the durable effect journal and approval
    /// repository.
    ///
    /// Returns an error instead of continuing with a partial recovery set: a
    /// journal or approval scan failure would otherwise drop in-flight
    /// model-agent effects while the runtime starts normally.
    pub(crate) fn plan_model_agent_recovery(
        &self,
        snapshot: &KernelState,
    ) -> Result<Vec<ModelAgentProposalRequest>, EffectFailure> {
        let mut proposals = BTreeMap::new();
        let mut approval_owned_runs = BTreeSet::new();
        let entries = self
            .adapters
            .effect_journal
            .scan_in_flight()
            .map_err(|error| {
                EffectFailure::Failed(format!(
                    "scan effect journal for model-agent recovery: {error}"
                ))
            })?;
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
        let records = self.adapters.approval_repo.find_all().map_err(|error| {
            EffectFailure::Failed(format!("scan approvals for model-agent recovery: {error}"))
        })?;
        for record in records {
            let proposal = crate::proposal_persistence::decode_pending_continuation(&record)
                .map_err(|error| {
                    EffectFailure::Failed(format!(
                        "decode approval {} for model-agent recovery: {error}",
                        record.id
                    ))
                })?;
            let Some(proposal) = proposal else {
                continue;
            };
            if !snapshot.model_agent_requests.contains_key(&proposal.run_id) {
                continue;
            }
            approval_owned_runs.insert(proposal.run_id);
            if !matches!(
                record.status,
                maestria_ports::ApprovalStatus::Approved | maestria_ports::ApprovalStatus::Denied
            ) || snapshot.model_agent_results.contains_key(&proposal.run_id)
            {
                continue;
            }
            proposals.entry(proposal.run_id).or_insert(proposal);
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
        Ok(proposals.into_values().collect())
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
                    MaestriaEffect::QueryHarnessProposal(Box::new(proposal)),
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
        generation: maestria_domain::JournalGeneration,
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
        Self::send_input(
            &self.input_tx,
            crate::harness::harness_completion_input(
                proposal.run_id,
                generation.value(),
                proposal.task_id,
                &outcome,
            ),
            "recovered harness completion",
        )
        .map_err(|error| {
            EffectFailure::Degraded(format!("deliver recovered harness result: {error}"))
        })?;
        Ok(Some(model_agent_harness_result(&outcome)))
    }
}
