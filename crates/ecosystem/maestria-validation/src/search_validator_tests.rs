use maestria_domain::{
    ClaimId, ConflictSet, ConflictSetId, EvidenceCandidate, EvidenceCandidateDto, EvidenceCoverage,
    EvidenceCoverageDto, EvidenceId, EvidenceRequirements, FreshnessStatus,
    SearchCompatibilityError, SearchStatus, ValidationReportId,
};

use super::{SEARCH_CHECKS, SearchCheck, ValidationRunner, Validator};

use crate::search_validator_fixtures::*;

fn validator_for(name: &str) -> SearchCheck {
    for check in SEARCH_CHECKS {
        if check.name == name {
            return SearchCheck {
                name: check.name,
                check: check.check,
            };
        }
    }
    SEARCH_CHECKS[0]
}

#[test]
fn all_search_validators_execute_and_pass_for_a_reproducible_outcome()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let validators: Vec<Box<dyn Validator>> = SEARCH_CHECKS
        .iter()
        .map(|c| {
            let sc = SearchCheck {
                name: c.name,
                check: c.check,
            };
            Box::new(sc) as Box<dyn Validator>
        })
        .collect();
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
fn invalid_plan_schema_is_rejected_at_construction() -> Result<(), Box<dyn std::error::Error>> {
    let plan = fixture()?.plan;
    let requirements = plan.evidence_requirements().clone();
    let invalid = plan.with_evidence_requirements(EvidenceRequirements {
        minimum_corroboration: 0,
        ..requirements
    });
    assert!(matches!(
        invalid,
        Err(SearchCompatibilityError::InvalidPlan(
            "minimum corroboration must be greater than 0"
        ))
    ));
    Ok(())
}

#[test]
fn candidate_provenance_validator_fails_for_missing_evidence_record()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    fixture.evidences.clear();
    let check = validator_for("candidate_provenance").validate(&fixture.context());
    assert!(!check.passed);
    assert!(
        check.message.contains("invalid evidence record")
            || check.message.contains("has no evidence record")
            || check.message.contains("provenance does not match")
    );
    Ok(())
}

#[test]
fn coverage_validator_fails_when_answerable_coverage_is_incomplete()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = fixture()?;
    let coverage = fixture.outcome.coverage.clone();
    fixture.outcome.coverage = EvidenceCoverage::new(EvidenceCoverageDto {
        percent_covered: 50,
        gaps_identified: coverage.gaps_identified().to_vec(),
        required_claims: coverage.required_claims().to_vec(),
        required_subquestions: coverage.required_subquestions().to_vec(),
        distinct_sources: coverage.distinct_sources(),
        distinct_documents: coverage.distinct_documents(),
        distinct_sections: coverage.distinct_sections(),
        candidate_coverage_keys: coverage.candidate_coverage_keys().to_vec(),
    })?;
    let check = validator_for("coverage").validate(&fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("Answerable"));
    Ok(())
}

#[test]
fn conflict_validator_fails_when_status_and_members_mismatch()
-> Result<(), Box<dyn std::error::Error>> {
    let mut conflict_fixture = fixture()?;
    conflict_fixture.outcome.status = SearchStatus::SourcesConflict;
    let check = validator_for("conflict").validate(&conflict_fixture.context());
    assert!(!check.passed);
    assert!(check.message.contains("disagree"));

    let mut unknown_fixture = fixture()?;
    let unknown_conflict = ConflictSet {
        id: ConflictSetId::new(10),
        candidates: vec![conflict_fixture.outcome.evidence[0].clone()],
    };
    unknown_fixture.outcome.status = SearchStatus::Answerable;
    unknown_fixture.outcome.conflicts.push(unknown_conflict);
    let check = validator_for("conflict").validate(&unknown_fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn freshness_validator_fails_for_stale_high_rank_evidence() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = fixture()?;
    if let Some(candidate) = fixture.outcome.evidence.first() {
        let rebuilt = EvidenceCandidate::new(EvidenceCandidateDto {
            evidence_id: candidate.evidence_id(),
            artifact_version: candidate.artifact_version(),
            source_span: candidate.source_span().clone(),
            scores: candidate.scores().clone(),
            trust: candidate.trust(),
            freshness: FreshnessStatus::Stale,
            duplicate_cluster: candidate.duplicate_cluster(),
            reasons: candidate.reasons().to_vec(),
            coverage_keys: candidate.coverage_keys().to_vec(),
        })?;
        fixture.outcome.evidence[0] = rebuilt;
    }
    let check = validator_for("freshness").validate(&fixture.context());
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
    let check = validator_for("citation_alignment").validate(&fixture.context());
    assert!(!check.passed);
    Ok(())
}

#[test]
fn search_regression_validator_fails_for_identity_and_duplicate_candidates()
-> Result<(), Box<dyn std::error::Error>> {
    let mut identity_fixture = fixture()?;
    identity_fixture.outcome.trace = maestria_domain::SearchTraceId::new(404);
    let check = validator_for("search_regression").validate(&identity_fixture.context());
    assert!(!check.passed);
    let mut duplicate_fixture = fixture()?;
    if let Some(first) = duplicate_fixture.outcome.evidence.first().cloned() {
        duplicate_fixture.outcome.evidence.push(first);
        let duplicate_check =
            validator_for("search_regression").validate(&duplicate_fixture.context());
        assert!(!duplicate_check.passed);
    }
    Ok(())
}
