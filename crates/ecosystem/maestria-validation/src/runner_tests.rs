use std::collections::{BTreeMap, BTreeSet};

use super::*;
use maestria_domain::{
    Artifact, ArtifactId, Claim, ClaimId, ClaimStatus, ContentRange, Evidence, EvidenceId,
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
