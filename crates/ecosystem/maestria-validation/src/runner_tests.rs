use super::test_fixtures::*;
use super::*;
use maestria_domain::{
    ClaimId, EvidenceId, MemoryCandidateId, TaskId, TaskStatus, ValidationReportId,
};

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
fn validation_runner_passes_when_all_default_checks_pass() -> Result<(), Box<dyn std::error::Error>>
{
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
        .insert(evidence_id, evidence(10, Some(claim_id))?);
    fixture.memory_candidates.insert(
        MemoryCandidateId::new(1),
        memory_candidate(1, [evidence_id])?,
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
    Ok(())
}

#[test]
fn validation_runner_reports_failures_as_errors_not_warnings()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ContextFixture {
        task: Some(task(1, TaskStatus::Active)),
        harness_exit_code: Some(1),
        ..ContextFixture::default()
    };
    fixture.claims.insert(ClaimId::new(1), claim(1, []));
    fixture.memory_candidates.insert(
        MemoryCandidateId::new(1),
        memory_candidate(1, [EvidenceId::new(20)])?,
    );

    let report = ValidationRunner::new().run(
        ValidationReportId::new(100),
        Some(TaskId::new(1)),
        &fixture.context(),
    );

    assert!(!report.passed);
    assert_eq!(report.checks.len(), 13);
    assert_eq!(report.warnings.len(), 0);
    Ok(())
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
