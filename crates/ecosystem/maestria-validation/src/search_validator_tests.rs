use maestria_domain::{
    ClaimId, ConflictSet, ConflictSetId, EvidenceId, FreshnessStatus, SearchStatus,
    ValidationReportId,
};

use super::{
    CandidateProvenanceValidator, CitationAlignmentValidator, ConflictValidator, CoverageValidator,
    FreshnessValidator, SearchPlanValidator, SearchRegressionValidator, ValidationRunner,
    Validator,
};

use crate::search_validator_fixtures::*;

#[test]
fn all_search_validators_execute_and_pass_for_a_reproducible_outcome()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let validators: Vec<Box<dyn Validator>> = vec![
        Box::new(SearchPlanValidator),
        Box::new(CandidateProvenanceValidator),
        Box::new(CoverageValidator),
        Box::new(ConflictValidator),
        Box::new(FreshnessValidator),
        Box::new(CitationAlignmentValidator),
        Box::new(super::RetrievalSecurityValidator),
        Box::new(SearchRegressionValidator),
    ];
    let report = ValidationRunner::with_validators(validators).run(
        ValidationReportId::new(1),
        None,
        &fixture.context(),
    );

    assert!(report.passed, "{:?}", report.checks);
    assert_eq!(report.checks.len(), 8);
    assert!(report.checks.iter().all(|check| check.passed));
    assert!(
        report
            .checks
            .iter()
            .all(|check| matches!(check.severity, crate::Severity::Error))
    );
    Ok(())
}

#[test]
fn search_plan_validator_fails_with_invalid_plan_schema() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    fixture.plan.evidence_requirements.minimum_corroboration = 0;
    let check = SearchPlanValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("minimum corroboration"));
    Ok(())
}

#[test]
fn candidate_provenance_validator_fails_for_missing_evidence_record()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    fixture.evidences.clear();
    let check = CandidateProvenanceValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("invalid evidence record"));
    Ok(())
}

#[test]
fn coverage_validator_fails_when_answerable_coverage_is_incomplete()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    fixture.outcome.coverage.percent_covered = 50;
    let check = CoverageValidator.validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("Answerable"));
    Ok(())
}

#[test]
fn conflict_validator_fails_when_status_and_members_mismatch()
-> Result<(), Box<dyn std::error::Error>> {
    let mut conflict_fixture = fixture()?;
    conflict_fixture.outcome.status = SearchStatus::SourcesConflict;
    let check = ConflictValidator.validate(&conflict_fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("disagree"));

    let mut unknown_fixture = fixture()?;
    let unknown_conflict = ConflictSet {
        id: ConflictSetId::new(10),
        candidates: vec![conflict_fixture.outcome.evidence[0].clone()],
    };
    unknown_fixture.outcome.status = SearchStatus::Answerable;
    unknown_fixture.outcome.conflicts.push(unknown_conflict);
    let check = ConflictValidator.validate(&unknown_fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn freshness_validator_fails_for_stale_high_rank_evidence() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    if let Some(candidate) = fixture.outcome.evidence.first_mut() {
        candidate.freshness = FreshnessStatus::Stale;
    }
    let check = FreshnessValidator.validate(&fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn citation_alignment_validator_fails_for_unbound_claims() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    fixture
        .claims
        .insert(ClaimId::new(1), claim(1, [EvidenceId::new(99)]));
    let check = CitationAlignmentValidator.validate(&fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn search_regression_validator_fails_for_identity_and_duplicate_candidates()
-> Result<(), Box<dyn std::error::Error>> {
    let mut identity_fixture = fixture()?;
    identity_fixture.outcome.trace = maestria_domain::SearchTraceId::new(404);
    let check = SearchRegressionValidator.validate(&identity_fixture.context());
    assert!(!check.passed);
    let mut duplicate_fixture = fixture()?;
    if let Some(first) = duplicate_fixture.outcome.evidence.first().cloned() {
        duplicate_fixture.outcome.evidence.push(first);
        let duplicate_check = SearchRegressionValidator.validate(&duplicate_fixture.context());
        assert!(!duplicate_check.passed);
    }
    Ok(())
}
