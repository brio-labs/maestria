use super::test_fixtures::*;
use super::*;
use maestria_domain::{
    ArtifactId, BlobId, Evidence, EvidenceId, EvidenceKind, LogicalTick, MemoryCandidateId,
    SecurityMetadata, ValidationReportId,
};

#[test]
fn memory_validator_passes_when_all_candidates_have_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ContextFixture::default();
    let evidence_id = EvidenceId::new(10);
    fixture.evidences.insert(evidence_id, evidence(10, None)?);
    fixture.memory_candidates.insert(
        MemoryCandidateId::new(1),
        memory_candidate(1, [evidence_id])?,
    );

    let check = MemoryValidator.validate(&fixture.context());

    assert!(check.passed);
    assert_eq!(check.name, "memory");
    Ok(())
}

#[test]
fn memory_validator_fails_when_candidate_references_missing_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ContextFixture::default();
    fixture.memory_candidates.insert(
        MemoryCandidateId::new(1),
        memory_candidate(1, [EvidenceId::new(10)])?,
    );

    let check = MemoryValidator.validate(&fixture.context());

    assert!(!check.passed);
    assert!(
        check
            .message
            .contains("1 memory candidate evidence reference")
    );
    Ok(())
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
