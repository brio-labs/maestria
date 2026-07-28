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
        let state = self.state.read().await;
        let Some(stored) = state.model_agent_requests.get(&proposal.run_id) else {
            tracing::warn!(
                approval_id = %decision.approval_id,
                run_id = %proposal.run_id,
                "approval resolve rejected: stored model-agent request missing"
            );
            return false;
        };
        let mut stored_identity = stored.clone();
        stored_identity.approval_id = proposal.approval_id;
        stored_identity.journal_generation = proposal.journal_generation;
        if stored_identity != proposal || proposal.approval_id != Some(decision.approval_id) {
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
