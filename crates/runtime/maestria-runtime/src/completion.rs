use crate::MaestriaRuntime;
use maestria_domain::{CompleteTaskInput, RunValidationRequest, TaskStatus};
use maestria_governance::{ValidationDecision, ValidationRequest};

impl MaestriaRuntime {
    pub(crate) async fn check_completion_validation(
        &self,
        complete_input: &CompleteTaskInput,
    ) -> bool {
        let state = self.state.read().await;
        let task = state.tasks.get(&complete_input.task_id).cloned();
        let recomputed_report = crate::validation::build_validation_report_from_state(
            &state,
            &RunValidationRequest {
                task_id: Some(complete_input.task_id),
                claim_id: None,
                validation_report_id: complete_input.validation_report_id,
            },
        );
        drop(state);

        let mut durable_report = None;
        match self
            .adapters
            .event_log
            .scan(maestria_ports::EventFilter { artifact_id: None })
        {
            Ok(events) => {
                for env in events {
                    if let maestria_domain::DomainEvent::ValidationReportCreated {
                        report_id,
                        task_id,
                        passed,
                        warnings,
                    } = &env.event
                        && *report_id == complete_input.validation_report_id
                        && *task_id == Some(complete_input.task_id)
                    {
                        durable_report = Some(maestria_domain::ValidationReportRecord {
                            task_id: *task_id,
                            passed: *passed,
                            warnings: warnings.clone(),
                        });
                        break;
                    }
                }
            }
            Err(e) => {
                tracing::error!(%e, "task completion blocked: failed to scan event log");
                return false;
            }
        }

        if durable_report.is_none() {
            tracing::warn!("task completion blocked: validation report not durable in event log");
            return false;
        }
        if !recomputed_report.passed {
            tracing::warn!("task completion blocked: current validation pass failed");
            return false;
        }

        if let Some(task) = task {
            let proposed_status = match &durable_report {
                Some(r) if !r.warnings.is_empty() => TaskStatus::CompletedWithWarnings,
                _ => TaskStatus::CompletedVerified,
            };
            let request = ValidationRequest {
                task,
                validation_report: durable_report,
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
