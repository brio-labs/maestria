use super::test_fixtures::*;
use super::*;
use maestria_domain::{
    ArtifactId, BlobId, ClaimId, Evidence, EvidenceId, EvidenceKind, LogicalTick,
    MemoryCandidateId, SecurityMetadata, TaskStatus, ValidationReportId,
};

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
fn memory_validator_passes_when_all_candidates_have_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ContextFixture::default();
    let evidence_id = EvidenceId::new(10);
    fixture.evidences.insert(evidence_id, evidence(10, None)?);
    fixture.memory_candidates.insert(
        MemoryCandidateId::new(1),
        memory_candidate(1, [evidence_id]),
    );

    let check = MemoryValidator.validate(&fixture.context());

    assert!(check.passed);
    assert_eq!(check.name, "memory");
    Ok(())
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
