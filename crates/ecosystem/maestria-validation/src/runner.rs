use maestria_domain::{TaskId, ValidationReportId};

use super::types::{Severity, ValidationCheck, ValidationContext, ValidationReport, Validator};
use super::validators::{
    CitationValidator, EvidenceExistenceValidator, HarnessRunValidator, MemoryValidator,
    TaskStateValidator,
};

pub struct ValidationRunner {
    validators: Vec<Box<dyn Validator>>,
}

impl ValidationRunner {
    pub fn new() -> Self {
        Self::with_validators(vec![
            Box::new(CitationValidator),
            Box::new(EvidenceExistenceValidator),
            Box::new(TaskStateValidator),
            Box::new(HarnessRunValidator),
            Box::new(MemoryValidator),
        ])
    }

    pub fn with_validators(validators: Vec<Box<dyn Validator>>) -> Self {
        Self { validators }
    }

    pub fn run(
        &self,
        report_id: ValidationReportId,
        task_id: Option<TaskId>,
        context: &ValidationContext<'_>,
    ) -> ValidationReport {
        let checks: Vec<ValidationCheck> = self
            .validators
            .iter()
            .map(|validator| validator.validate(context))
            .collect();
        let passed = checks
            .iter()
            .all(|check| check.passed || check.severity != Severity::Error);
        let warnings = checks
            .iter()
            .filter(|check| !check.passed && check.severity == Severity::Warning)
            .map(|check| check.message.clone())
            .collect();

        ValidationReport {
            id: report_id,
            task_id,
            checks,
            passed,
            warnings,
        }
    }
}

impl Default for ValidationRunner {
    fn default() -> Self {
        Self::new()
    }
}
