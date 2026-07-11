use maestria_domain::Task;

/// Request to evaluate whether a task's completion is valid.
#[derive(Debug)]
pub struct ValidationRequest {
    pub task: Task,
    pub validation_report_present: bool,
    pub had_warning: bool,
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
        if !request.validation_report_present {
            return ValidationDecision::BlockedByMissingValidation {
                reason: "task completion requires validation report".to_string(),
            };
        }

        if request.task.status.is_completion() {
            if request.had_warning && !self.allow_warnings {
                return ValidationDecision::BlockedByPolicy {
                    reason: "warnings are blocked in this policy".to_string(),
                };
            }
            ValidationDecision::AllowCompletion
        } else {
            ValidationDecision::BlockedByPolicy {
                reason: format!(
                    "task status {:?} is not completion state",
                    request.task.status
                ),
            }
        }
    }
}
