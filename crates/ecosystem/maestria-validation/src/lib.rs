#![forbid(unsafe_code)]

//! Pure validation mechanisms for Maestria domain snapshots.
//!
//! This crate owns validation checks only. It does not decide policy, perform I/O,
//! or persist reports; callers provide an immutable domain snapshot and receive a
//! deterministic [`ValidationReport`].

use std::collections::BTreeMap;

use maestria_domain::{
    Claim, ClaimId, Evidence, EvidenceId, MemoryCandidate, MemoryCandidateId, Task, TaskId,
    TaskStatus, ValidationReportId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub id: ValidationReportId,
    pub task_id: Option<TaskId>,
    pub checks: Vec<ValidationCheck>,
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

pub trait Validator: Send + Sync {
    fn name(&self) -> &str;
    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck;
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationContext<'a> {
    pub task: Option<&'a Task>,
    pub claims: &'a BTreeMap<ClaimId, Claim>,
    pub evidences: &'a BTreeMap<EvidenceId, Evidence>,
    pub memory_candidates: &'a BTreeMap<MemoryCandidateId, MemoryCandidate>,
    pub harness_exit_code: Option<i32>,
}

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
        let missing_count = context
            .claims
            .values()
            .flat_map(|claim| claim.evidence_ids.iter())
            .filter(|evidence_id| !context.evidences.contains_key(evidence_id))
            .count();

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
        let (missing_evidence_refs, missing_candidate_ids) =
            context.memory_candidates.values().fold(
                (0usize, 0usize),
                |(missing_refs, missing_ids), candidate| {
                    let missing_ids = missing_ids + usize::from(candidate.evidence_ids.is_empty());
                    let refs = candidate
                        .evidence_ids
                        .iter()
                        .filter(|evidence_id| !context.evidences.contains_key(evidence_id))
                        .count();
                    (missing_refs + refs, missing_ids)
                },
            );

        if missing_evidence_refs == 0 && missing_candidate_ids == 0 {
            passed_check(
                self.name(),
                "all memory candidates have at least one referenced evidence id",
            )
        } else {
            let mut messages = Vec::new();
            if missing_candidate_ids > 0 {
                messages.push(format!(
                    "{missing_candidate_ids} memory candidate(s) are missing evidence ids"
                ));
            }
            if missing_evidence_refs > 0 {
                messages.push(format!(
                    "{missing_evidence_refs} memory candidate evidence reference(s) do not exist"
                ));
            }
            failed_check(self.name(), messages.join("; "))
        }
    }
}

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
        let passed = checks.iter().all(|check| check.passed);
        let warnings = checks
            .iter()
            .filter(|check| !check.passed)
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

fn passed_check(name: &str, message: &str) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: true,
        message: message.to_string(),
    }
}

fn failed_check(name: &str, message: impl Into<String>) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: false,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use maestria_domain::{
        ArtifactId, BlobId, ClaimStatus, ContentRange, EvidenceKind, LogicalTick, TaskPriority,
    };

    use super::*;

    #[derive(Default)]
    struct ContextFixture {
        task: Option<Task>,
        claims: BTreeMap<ClaimId, Claim>,
        evidences: BTreeMap<EvidenceId, Evidence>,
        memory_candidates: BTreeMap<MemoryCandidateId, MemoryCandidate>,
        harness_exit_code: Option<i32>,
    }

    impl ContextFixture {
        fn context(&self) -> ValidationContext<'_> {
            ValidationContext {
                task: self.task.as_ref(),
                claims: &self.claims,
                evidences: &self.evidences,
                memory_candidates: &self.memory_candidates,
                harness_exit_code: self.harness_exit_code,
            }
        }
    }

    fn claim(id: u64, evidence_ids: impl IntoIterator<Item = EvidenceId>) -> Claim {
        Claim {
            id: ClaimId::new(id),
            artifact_id: ArtifactId::new(id),
            text: format!("claim {id}"),
            status: ClaimStatus::Proposed,
            evidence_ids: evidence_ids.into_iter().collect(),
        }
    }

    fn evidence(id: u64, claim_id: Option<ClaimId>) -> Evidence {
        Evidence {
            id: EvidenceId::new(id),
            artifact_id: ArtifactId::new(1),
            claim_id,
            kind: EvidenceKind::FileSpan {
                path: "src/lib.rs".to_string(),
                range: ContentRange { start: 0, end: 8 },
                content_hash: "hash".to_string(),
            },
            excerpt: "evidence excerpt".to_string(),
            observed_at: LogicalTick::new(1),
        }
    }

    fn task(id: u64, status: TaskStatus) -> Task {
        Task {
            id: TaskId::new(id),
            title: format!("task {id}"),
            priority: TaskPriority::Normal,
            status,
            validation_report_id: None,
            artifact_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        }
    }

    fn memory_candidate(
        id: u64,
        evidence_ids: impl IntoIterator<Item = EvidenceId>,
    ) -> MemoryCandidate {
        MemoryCandidate {
            id: MemoryCandidateId::new(id),
            claim_id: ClaimId::new(id),
            evidence_ids: evidence_ids.into_iter().collect(),
            confidence_milli: 900,
        }
    }

    #[test]
    fn citation_validator_passes_when_all_claims_have_evidence() {
        let mut fixture = ContextFixture::default();
        fixture
            .claims
            .insert(ClaimId::new(1), claim(1, [EvidenceId::new(10)]));

        let check = CitationValidator.validate(&fixture.context());

        assert!(check.passed);
        assert_eq!(check.name, "citation");
    }

    #[test]
    fn citation_validator_fails_when_any_claim_lacks_evidence() {
        let mut fixture = ContextFixture::default();
        fixture.claims.insert(ClaimId::new(1), claim(1, []));
        fixture
            .claims
            .insert(ClaimId::new(2), claim(2, [EvidenceId::new(20)]));

        let check = CitationValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check.message.contains("1 claim"));
    }

    #[test]
    fn evidence_existence_validator_passes_when_claim_references_exist() {
        let mut fixture = ContextFixture::default();
        let claim_id = ClaimId::new(1);
        let evidence_id = EvidenceId::new(10);
        fixture.claims.insert(claim_id, claim(1, [evidence_id]));
        fixture
            .evidences
            .insert(evidence_id, evidence(10, Some(claim_id)));

        let check = EvidenceExistenceValidator.validate(&fixture.context());

        assert!(check.passed);
        assert_eq!(check.name, "evidence_existence");
    }

    #[test]
    fn evidence_existence_validator_fails_when_claim_reference_is_missing() {
        let mut fixture = ContextFixture::default();
        fixture
            .claims
            .insert(ClaimId::new(1), claim(1, [EvidenceId::new(404)]));

        let check = EvidenceExistenceValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check.message.contains("1 claim evidence reference"));
    }

    #[test]
    fn task_state_validator_passes_for_validating_task() {
        let fixture = ContextFixture {
            task: Some(task(1, TaskStatus::Validating)),
            ..ContextFixture::default()
        };

        let check = TaskStateValidator.validate(&fixture.context());

        assert!(check.passed);
        assert_eq!(check.name, "task_state");
    }

    #[test]
    fn task_state_validator_fails_for_non_validating_task() {
        let fixture = ContextFixture {
            task: Some(task(1, TaskStatus::Active)),
            ..ContextFixture::default()
        };

        let check = TaskStateValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check.message.contains("Validating"));
    }

    #[test]
    fn task_state_validator_fails_without_task() {
        let fixture = ContextFixture::default();

        let check = TaskStateValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check.message.contains("task is required"));
    }

    #[test]
    fn harness_run_validator_passes_for_successful_exit_code() {
        let fixture = ContextFixture {
            harness_exit_code: Some(0),
            ..ContextFixture::default()
        };

        let check = HarnessRunValidator.validate(&fixture.context());

        assert!(check.passed);
        assert_eq!(check.name, "harness_run");
    }

    #[test]
    fn harness_run_validator_passes_when_no_exit_code_is_present() {
        let fixture = ContextFixture::default();

        let check = HarnessRunValidator.validate(&fixture.context());

        assert!(check.passed);
        assert!(check.message.contains("no harness run"));
    }

    #[test]
    fn harness_run_validator_fails_for_non_zero_exit_code() {
        let fixture = ContextFixture {
            harness_exit_code: Some(2),
            ..ContextFixture::default()
        };

        let check = HarnessRunValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check.message.contains("2"));
    }

    #[test]
    fn memory_validator_passes_when_all_candidates_have_evidence() {
        let mut fixture = ContextFixture::default();
        let evidence_id = EvidenceId::new(10);
        fixture.evidences.insert(evidence_id, evidence(10, None));
        fixture.memory_candidates.insert(
            MemoryCandidateId::new(1),
            memory_candidate(1, [evidence_id]),
        );

        let check = MemoryValidator.validate(&fixture.context());

        assert!(check.passed);
        assert_eq!(check.name, "memory");
    }

    #[test]
    fn memory_validator_fails_when_any_candidate_lacks_evidence() {
        let mut fixture = ContextFixture::default();
        fixture
            .memory_candidates
            .insert(MemoryCandidateId::new(1), memory_candidate(1, []));
        fixture.memory_candidates.insert(
            MemoryCandidateId::new(2),
            memory_candidate(2, [EvidenceId::new(20)]),
        );

        let check = MemoryValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check.message.contains("1 memory candidate"));
    }
    #[test]
    fn memory_validator_fails_when_candidate_references_missing_evidence() {
        let mut fixture = ContextFixture::default();
        fixture.memory_candidates.insert(
            MemoryCandidateId::new(1),
            memory_candidate(1, [EvidenceId::new(10)]),
        );

        let check = MemoryValidator.validate(&fixture.context());

        assert!(!check.passed);
        assert!(check
            .message
            .contains("1 memory candidate evidence reference"));
    }

    #[test]
    fn validation_runner_passes_when_all_default_checks_pass() {
        let mut fixture = ContextFixture {
            task: Some(task(1, TaskStatus::Validating)),
            harness_exit_code: Some(0),
            ..ContextFixture::default()
        };
        let claim_id = ClaimId::new(1);
        let evidence_id = EvidenceId::new(10);
        fixture.claims.insert(claim_id, claim(1, [evidence_id]));
        fixture
            .evidences
            .insert(evidence_id, evidence(10, Some(claim_id)));
        fixture.memory_candidates.insert(
            MemoryCandidateId::new(1),
            memory_candidate(1, [evidence_id]),
        );

        let report = ValidationRunner::new().run(
            ValidationReportId::new(99),
            Some(TaskId::new(1)),
            &fixture.context(),
        );

        assert!(report.passed);
        assert_eq!(report.id, ValidationReportId::new(99));
        assert_eq!(report.task_id, Some(TaskId::new(1)));
        assert_eq!(report.checks.len(), 5);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn validation_runner_collects_failures_as_warnings() {
        let mut fixture = ContextFixture {
            task: Some(task(1, TaskStatus::Active)),
            harness_exit_code: Some(1),
            ..ContextFixture::default()
        };
        fixture.claims.insert(ClaimId::new(1), claim(1, []));
        fixture
            .memory_candidates
            .insert(MemoryCandidateId::new(1), memory_candidate(1, []));

        let report = ValidationRunner::new().run(
            ValidationReportId::new(100),
            Some(TaskId::new(1)),
            &fixture.context(),
        );

        assert!(!report.passed);
        assert_eq!(report.checks.len(), 5);
        assert_eq!(report.warnings.len(), 4);
        assert!(report
            .warnings
            .iter()
            .any(|message| message.contains("claim")));
        assert!(report
            .warnings
            .iter()
            .any(|message| message.contains("Validating")));
        assert!(report
            .warnings
            .iter()
            .any(|message| message.contains("non-zero")));
        assert!(report
            .warnings
            .iter()
            .any(|message| message.contains("memory candidate")));
    }

    #[test]
    fn validation_runner_accepts_custom_validator_list() {
        let fixture = ContextFixture::default();
        let runner = ValidationRunner::with_validators(vec![Box::new(CitationValidator)]);

        let report = runner.run(ValidationReportId::new(1), None, &fixture.context());

        assert!(report.passed);
        assert_eq!(report.checks.len(), 1);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn empty_context_passes_collection_validators_and_harness_validator() {
        let fixture = ContextFixture::default();
        let context = fixture.context();

        let citation = CitationValidator.validate(&context);
        let evidence = EvidenceExistenceValidator.validate(&context);
        let harness = HarnessRunValidator.validate(&context);
        let memory = MemoryValidator.validate(&context);

        assert!(citation.passed);
        assert!(evidence.passed);
        assert!(harness.passed);
        assert!(memory.passed);
    }

    #[test]
    fn evidence_test_helper_uses_blob_type_for_validation_variant_coverage() {
        let validation_evidence = Evidence {
            id: EvidenceId::new(70),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(7),
            },
            excerpt: format!("blob {}", BlobId::new(3)),
            observed_at: LogicalTick::new(1),
        };

        assert_eq!(validation_evidence.id, EvidenceId::new(70));
    }
}
