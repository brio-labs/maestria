use maestria_domain::TaskStatus;

use super::types::{
    ValidationCheck, ValidationContext, Validator, count_missing_evidence, failed_check,
    passed_check,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct CitationValidator;

impl Validator for CitationValidator {
    fn name(&self) -> &str {
        "citation"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        let missing_count = context
            .claims
            .values()
            .filter(|claim| claim.evidence_ids.is_empty())
            .count();

        if missing_count == 0 {
            passed_check(
                self.name(),
                "all claims have at least one linked evidence id",
            )
        } else {
            failed_check(
                self.name(),
                format!("{missing_count} claim(s) are missing linked evidence ids"),
            )
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EvidenceExistenceValidator;

impl Validator for EvidenceExistenceValidator {
    fn name(&self) -> &str {
        "evidence_existence"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        let missing_count = count_missing_evidence(
            context
                .claims
                .values()
                .flat_map(|claim| claim.evidence_ids.iter().copied()),
            context.evidences,
        );

        if missing_count == 0 {
            passed_check(
                self.name(),
                "all claim evidence references resolve to evidence records",
            )
        } else {
            failed_check(
                self.name(),
                format!("{missing_count} claim evidence reference(s) do not exist"),
            )
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TaskStateValidator;

impl Validator for TaskStateValidator {
    fn name(&self) -> &str {
        "task_state"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        match context.task {
            Some(task) if task.status == TaskStatus::Validating => {
                passed_check(self.name(), "task is in validating status")
            }
            Some(task) => failed_check(
                self.name(),
                format!(
                    "task {} must be Validating before completion, found {:?}",
                    task.id, task.status
                ),
            ),
            None => failed_check(self.name(), "task is required for task-state validation"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HarnessRunValidator;

impl Validator for HarnessRunValidator {
    fn name(&self) -> &str {
        "harness_run"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        match context.harness_exit_code {
            Some(0) => passed_check(self.name(), "harness run exited successfully"),
            Some(code) => failed_check(
                self.name(),
                format!("harness run exited with non-zero code {code}"),
            ),
            None => passed_check(self.name(), "no harness run exit code was provided"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryValidator;

impl Validator for MemoryValidator {
    fn name(&self) -> &str {
        "memory"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        let missing_evidence_refs = count_missing_evidence(
            context
                .memory_candidates
                .values()
                .flat_map(|candidate| candidate.evidence_ids().iter().copied()),
            context.evidences,
        );

        if missing_evidence_refs == 0 {
            passed_check(
                self.name(),
                "all memory candidates have at least one referenced evidence id",
            )
        } else {
            failed_check(
                self.name(),
                format!(
                    "{missing_evidence_refs} memory candidate evidence reference(s) do not exist"
                ),
            )
        }
    }
}
