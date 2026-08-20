use maestria_domain::{TaskId, ValidationReportId};

use super::search_validators::{SEARCH_CHECKS, SearchCheck};
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
        let mut validators: Vec<Box<dyn Validator>> = vec![
            Box::new(CitationValidator),
            Box::new(EvidenceExistenceValidator),
            Box::new(TaskStateValidator),
            Box::new(HarnessRunValidator),
            Box::new(MemoryValidator),
        ];
        for check in SEARCH_CHECKS {
            // SearchCheck is 'static and implements Validator; clone the struct (Copy) into a box.
            let boxed: Box<dyn Validator> = Box::new(SearchCheck {
                name: check.name,
                check: check.check,
            });
            validators.push(boxed);
        }
        Self::with_validators(validators)
    }

    pub fn for_target(has_task: bool, has_search: bool) -> Self {
        let mut validators: Vec<Box<dyn Validator>> = vec![
            Box::new(CitationValidator),
            Box::new(EvidenceExistenceValidator),
            Box::new(MemoryValidator),
        ];
        if has_task {
            validators.push(Box::new(TaskStateValidator));
            validators.push(Box::new(HarnessRunValidator));
        }
        if has_search {
            for check in SEARCH_CHECKS {
                validators.push(Box::new(SearchCheck {
                    name: check.name,
                    check: check.check,
                }));
            }
        } else {
            // Still include non-search validators that are always needed
            // TaskState/Harness already handled; no search checks.
        }
        Self::with_validators(validators)
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
