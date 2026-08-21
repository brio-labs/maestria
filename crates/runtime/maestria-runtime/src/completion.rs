use crate::MaestriaRuntime;
use maestria_domain::{
    CompleteTaskInput, RunValidationRequest, Task, ValidationReportId, ValidationReportRecord,
};
use maestria_governance::{ProposedCompletion, ValidationDecision, ValidationRequest};

impl MaestriaRuntime {
    pub(crate) async fn check_completion_validation(
        &self,
        complete_input: &CompleteTaskInput,
    ) -> bool {
        let (task, recomputed_passed) = self.recompute_task_validation(complete_input).await;
        let Some(durable_report) = self.find_durable_validation_report(
            complete_input.validation_report_id,
            complete_input.task_id,
        ) else {
            return false;
        };

        if !recomputed_passed {
            tracing::warn!("task completion blocked: current validation pass failed");
            return false;
        }

        let Some(task) = task else {
            // Task missing; allow domain to handle and reject it with MissingTask.
            return true;
        };

        self.evaluate_governance_validation(task, durable_report)
    }

    async fn recompute_task_validation(
        &self,
        complete_input: &CompleteTaskInput,
    ) -> (Option<Task>, bool) {
        let state = self.state.read().await;
        let task = state.tasks.get(&complete_input.task_id).cloned();
        let recomputed_report = crate::validation::build_validation_report_from_state(
            &state,
            &RunValidationRequest::for_task(
                complete_input.task_id,
                complete_input.validation_report_id,
            ),
        );
        (task, recomputed_report.passed)
    }

    fn find_durable_validation_report(
        &self,
        expected_report_id: ValidationReportId,
        expected_task_id: maestria_domain::TaskId,
    ) -> Option<ValidationReportRecord> {
        let events = match self
            .adapters
            .event_log
            .scan(maestria_ports::EventFilter { artifact_id: None })
        {
            Ok(events) => events,
            Err(error) => {
                tracing::error!(%error, "task completion blocked: failed to scan event log");
                return None;
            }
        };

        for env in events {
            if let maestria_domain::DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } = env.event
                && report_id == expected_report_id
                && task_id == Some(expected_task_id)
            {
                return Some(ValidationReportRecord {
                    task_id: Some(expected_task_id),
                    passed,
                    warnings,
                });
            }
        }

        tracing::warn!("task completion blocked: validation report not durable in event log");
        None
    }

    fn evaluate_governance_validation(
        &self,
        task: Task,
        durable_report: ValidationReportRecord,
    ) -> bool {
        let proposed_status = if durable_report.warnings.is_empty() {
            ProposedCompletion::Verified
        } else {
            ProposedCompletion::WithWarnings
        };
        let request = ValidationRequest {
            task,
            validation_report: Some(durable_report),
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
    }
}
