use crate::MaestriaRuntime;
use maestria_domain::ApprovalDecision;
use maestria_ports::ApprovalStatus;

impl MaestriaRuntime {
    pub(crate) async fn check_approval_boundary(&self, decision: &ApprovalDecision) -> bool {
        let record = match self.adapters.approval_repo.find_by_id(decision.approval_id) {
            Ok(None) => {
                tracing::warn!(
                    approval_id = %decision.approval_id,
                    "approval resolve rejected: record not found"
                );
                return false;
            }
            Ok(Some(record)) => record,
            Err(e) => {
                tracing::error!(
                    %e,
                    approval_id = %decision.approval_id,
                    "approval resolve rejected: repo lookup error"
                );
                return false;
            }
        };
        if record.status != ApprovalStatus::Pending {
            tracing::info!(
                approval_id = %decision.approval_id,
                status = ?record.status,
                "approval resolve skipped: already resolved (idempotent)"
            );
            return false;
        }
        if record.task_id != decision.task_id {
            tracing::warn!(
                approval_id = %decision.approval_id,
                record_task = ?record.task_id,
                input_task = ?decision.task_id,
                "approval resolve rejected: task_id mismatch"
            );
            return false;
        }
        let Some(proposal) = crate::effect_execution::decode_pending_continuation(&record) else {
            return true;
        };
        let maestria_domain::ModelAgentProposalExecution::ApprovalContinuation {
            approval_id,
            journal_generation: _,
        } = &proposal.execution
        else {
            tracing::warn!(
                approval_id = %decision.approval_id,
                "approval resolve rejected: continuation has invalid execution mode"
            );
            return false;
        };
        if *approval_id != decision.approval_id
            || record.effect_kind != "model_agent_harness"
            || record.scope_id != self.config.scope_id
            || record
                .capability
                .strip_prefix("model_agent_pending:")
                .is_none()
        {
            tracing::warn!(
                approval_id = %decision.approval_id,
                "approval resolve rejected: stored approval metadata mismatch"
            );
            return false;
        }
        let state = self.state.read().await;
        let Some(stored) = state.model_agent_requests.get(&proposal.run_id) else {
            tracing::warn!(
                approval_id = %decision.approval_id,
                run_id = %proposal.run_id,
                "approval resolve rejected: stored model-agent request missing"
            );
            return false;
        };
        if !matches!(
            &stored.execution,
            maestria_domain::ModelAgentProposalExecution::Fresh
        ) {
            tracing::warn!(
                approval_id = %decision.approval_id,
                run_id = %proposal.run_id,
                "approval resolve rejected: stored request is not fresh"
            );
            return false;
        }
        let mut expected = stored.clone();
        expected.execution = proposal.execution.clone();
        if expected != proposal {
            tracing::warn!(
                approval_id = %decision.approval_id,
                run_id = %proposal.run_id,
                "approval resolve rejected: continuation identity mismatch"
            );
            return false;
        }
        true
    }
}
