use std::collections::{BTreeMap, BTreeSet};

use super::*;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, Claim, ClaimId, ClaimStatus, ContentRange, Evidence, EvidenceId,
    EvidenceKind, LogicalTick, MemoryCandidate, MemoryCandidateId, SecurityMetadata, Task, TaskId,
    TaskPriority, TaskStatus, ValidationReportId,
};

#[derive(Default)]
struct ContextFixture {
    task: Option<Task>,
    artifacts: BTreeMap<ArtifactId, Artifact>,
    claims: BTreeMap<ClaimId, Claim>,
    evidences: BTreeMap<EvidenceId, Evidence>,
    memory_candidates: BTreeMap<MemoryCandidateId, MemoryCandidate>,
    harness_exit_code: Option<i32>,
}

impl ContextFixture {
    fn context(&self) -> ValidationContext<'_> {
        ValidationContext {
            task: self.task.as_ref(),
            artifacts: &self.artifacts,
            claims: &self.claims,
            evidences: &self.evidences,
            memory_candidates: &self.memory_candidates,
            harness_exit_code: self.harness_exit_code,
            search: None,
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
        security: SecurityMetadata::default(),
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
            snapshot: None,
        },
        excerpt: "evidence excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: SecurityMetadata::default(),
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
        security: SecurityMetadata::default(),
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
    assert!(
        check
            .message
            .contains("1 memory candidate evidence reference")
    );
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
    assert_eq!(report.checks.len(), 13);
    assert!(report.warnings.is_empty());
}

#[test]
fn validation_runner_reports_failures_as_errors_not_warnings() {
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
    assert_eq!(report.checks.len(), 13);
    assert_eq!(report.warnings.len(), 0);
}

struct DummyWarningValidator;
impl Validator for DummyWarningValidator {
    fn name(&self) -> &str {
        "dummy_warning"
    }
    fn validate(&self, _context: &ValidationContext<'_>) -> ValidationCheck {
        ValidationCheck {
            name: self.name().to_string(),
            passed: false,
            severity: super::types::Severity::Warning,
            message: "This is a warning".to_string(),
        }
    }
}

struct DummyErrorValidator;
impl Validator for DummyErrorValidator {
    fn name(&self) -> &str {
        "dummy_error"
    }
    fn validate(&self, _context: &ValidationContext<'_>) -> ValidationCheck {
        ValidationCheck {
            name: self.name().to_string(),
            passed: false,
            severity: super::types::Severity::Error,
            message: "This is an error".to_string(),
        }
    }
}

#[test]
fn validation_runner_passes_with_warnings() {
    let fixture = ContextFixture::default();
    let runner = ValidationRunner::with_validators(vec![Box::new(DummyWarningValidator)]);

    let report = runner.run(
        ValidationReportId::new(100),
        Some(TaskId::new(1)),
        &fixture.context(),
    );

    assert!(report.passed);
    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0], "This is a warning");
}

#[test]
fn validation_runner_fails_with_errors() {
    let fixture = ContextFixture::default();
    let runner = ValidationRunner::with_validators(vec![
        Box::new(DummyWarningValidator),
        Box::new(DummyErrorValidator),
    ]);

    let report = runner.run(
        ValidationReportId::new(100),
        Some(TaskId::new(1)),
        &fixture.context(),
    );

    assert!(!report.passed);
    assert_eq!(report.checks.len(), 2);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0], "This is a warning");
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
        security: SecurityMetadata::default(),
    };

    assert_eq!(validation_evidence.id, EvidenceId::new(70));
}
