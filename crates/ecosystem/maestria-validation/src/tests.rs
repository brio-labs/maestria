use super::test_fixtures::*;
use super::*;
use maestria_domain::{ClaimId, EvidenceId};

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
fn evidence_existence_validator_passes_when_claim_references_exist()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ContextFixture::default();
    let claim_id = ClaimId::new(1);
    let evidence_id = EvidenceId::new(10);
    fixture.claims.insert(claim_id, claim(1, [evidence_id]));
    fixture
        .evidences
        .insert(evidence_id, evidence(10, Some(claim_id))?);

    let check = EvidenceExistenceValidator.validate(&fixture.context());

    assert!(check.passed);
    assert_eq!(check.name, "evidence_existence");
    Ok(())
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
