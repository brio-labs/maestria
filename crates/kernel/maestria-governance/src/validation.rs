use maestria_domain::{Task, TaskStatus, ValidationReportRecord};

/// Request to evaluate whether a task's completion is valid.
#[derive(Debug)]
pub struct ValidationRequest {
    pub task: Task,
    pub validation_report: Option<ValidationReportRecord>,
    pub proposed_status: TaskStatus,
}

/// Outcome of a validation gate evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDecision {
    AllowCompletion,
    BlockedByMissingValidation { reason: String },
    BlockedByPolicy { reason: String },
}

/// Gate that decides whether a task completion is valid.
pub trait ValidationGate {
    fn evaluate(&self, request: &ValidationRequest) -> ValidationDecision;
}

/// Default validation gate.
#[derive(Debug)]
pub struct DefaultValidationGate {
    allow_warnings: bool,
}

impl DefaultValidationGate {
    pub const fn new(allow_warnings: bool) -> Self {
        Self { allow_warnings }
    }
}

impl ValidationGate for DefaultValidationGate {
    fn evaluate(&self, request: &ValidationRequest) -> ValidationDecision {
        if request.task.status != TaskStatus::Validating {
            return ValidationDecision::BlockedByPolicy {
                reason: format!(
                    "Task must be in Validating state to be completed, found {:?}",
                    request.task.status
                ),
            };
        }

        let report = match &request.validation_report {
            Some(r) => r,
            None => {
                return ValidationDecision::BlockedByMissingValidation {
                    reason: "task completion requires validation report".to_string(),
                };
            }
        };

        if report.task_id != Some(request.task.id) {
            return ValidationDecision::BlockedByPolicy {
                reason: "validation report does not match task id".to_string(),
            };
        }

        if !report.passed {
            return ValidationDecision::BlockedByPolicy {
                reason: "validation report indicates failure".to_string(),
            };
        }

        let has_warnings = !report.warnings.is_empty();

        if has_warnings && !self.allow_warnings {
            return ValidationDecision::BlockedByPolicy {
                reason: "warnings are blocked in this policy".to_string(),
            };
        }

        match request.proposed_status {
            TaskStatus::CompletedVerified => {
                if has_warnings {
                    ValidationDecision::BlockedByPolicy {
                        reason: "proposed status CompletedVerified but warnings are present"
                            .to_string(),
                    }
                } else {
                    ValidationDecision::AllowCompletion
                }
            }
            TaskStatus::CompletedWithWarnings => {
                if !has_warnings {
                    ValidationDecision::BlockedByPolicy {
                        reason: "proposed status CompletedWithWarnings but no warnings are present"
                            .to_string(),
                    }
                } else {
                    ValidationDecision::AllowCompletion
                }
            }
            _ => ValidationDecision::BlockedByPolicy {
                reason: format!(
                    "proposed status {:?} is not a valid successful completion status",
                    request.proposed_status
                ),
            },
        }
    }
}
