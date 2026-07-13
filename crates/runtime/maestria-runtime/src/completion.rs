use crate::MaestriaRuntime;
use maestria_domain::{CompleteTaskInput, TaskStatus};
use maestria_governance::{ValidationDecision, ValidationRequest};

impl MaestriaRuntime {
    pub(crate) async fn check_completion_validation(
        &self,
        complete_input: &CompleteTaskInput,
    ) -> bool {
        let state = self.state.read().await;
        let task = state.tasks.get(&complete_input.task_id).cloned();
        let report = state
            .validation_reports
            .get(&complete_input.validation_report_id)
            .cloned();
        drop(state);

        if let Some(task) = task {
            let proposed_status = match &report {
                Some(r) if !r.warnings.is_empty() => TaskStatus::CompletedWithWarnings,
                _ => TaskStatus::CompletedVerified,
            };
            let request = ValidationRequest {
                task,
                validation_report: report,
                proposed_status,
            };
            match self.governance.validation_gate.evaluate(&request) {
                ValidationDecision::AllowCompletion => true,
                ValidationDecision::BlockedByMissingValidation { reason } => {
                    tracing::warn!(%reason, "task completion blocked by missing validation");
                    false
                }
                ValidationDecision::BlockedByPolicy { reason } => {
                    tracing::warn!(%reason, "task completion blocked by governance policy");
                    false
                }
            }
        } else {
            // Task missing; allow domain to handle and reject it with MissingTask.
            true
        }
    }
}
