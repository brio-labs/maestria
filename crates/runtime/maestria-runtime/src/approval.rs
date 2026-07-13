use crate::MaestriaRuntime;
use maestria_domain::ApprovalDecision;
use maestria_ports::ApprovalStatus;

impl MaestriaRuntime {
    pub(crate) fn check_approval_boundary(&self, decision: &ApprovalDecision) -> bool {
        match self.adapters.approval_repo.find_by_id(decision.approval_id) {
            Ok(None) => {
                tracing::warn!(
                    approval_id = %decision.approval_id,
                    "approval resolve rejected: record not found"
                );
                false
            }
            Ok(Some(record)) if record.status != ApprovalStatus::Pending => {
                tracing::info!(
                    approval_id = %decision.approval_id,
                    status = ?record.status,
                    "approval resolve skipped: already resolved (idempotent)"
                );
                false
            }
            Ok(Some(record)) if record.task_id != decision.task_id => {
                tracing::warn!(
                    approval_id = %decision.approval_id,
                    record_task = %record.task_id,
                    input_task = %decision.task_id,
                    "approval resolve rejected: task_id mismatch"
                );
                false
            }
            Ok(Some(_)) => true,
            Err(e) => {
                tracing::error!(
                    %e,
                    approval_id = %decision.approval_id,
                    "approval resolve rejected: repo lookup error"
                );
                false
            }
        }
    }
}
