use std::collections::BTreeSet;

use maestria_domain::{
    ClaimId, DomainError, EvidenceId, MemoryCandidate, MemoryCandidateId, SecurityMetadata,
};

fn candidate(
    evidence_ids: BTreeSet<EvidenceId>,
    confidence_milli: u16,
) -> Result<MemoryCandidate, DomainError> {
    MemoryCandidate::try_new(
        MemoryCandidateId::new(1),
        ClaimId::new(2),
        evidence_ids,
        confidence_milli,
        SecurityMetadata::default(),
    )
}

#[test]
fn candidate_requires_evidence_at_construction() -> Result<(), Box<dyn std::error::Error>> {
    let error = candidate(BTreeSet::new(), 500)
        .err()
        .ok_or("empty evidence unexpectedly constructed a candidate")?;

    assert!(matches!(
        error,
        DomainError::MemoryCandidateRequiresEvidence { id } if id.value() == 1
    ));
    Ok(())
}

#[test]
fn candidate_rejects_out_of_range_confidence_at_construction()
-> Result<(), Box<dyn std::error::Error>> {
    let error = candidate(BTreeSet::from([EvidenceId::new(3)]), 1001)
        .err()
        .ok_or("out-of-range confidence unexpectedly constructed a candidate")?;

    assert!(matches!(
        error,
        DomainError::InvalidConfidence {
            max: 1000,
            actual: 1001,
        }
    ));
    Ok(())
}

#[test]
fn valid_candidate_exposes_only_read_accessors() -> Result<(), Box<dyn std::error::Error>> {
    let candidate = candidate(BTreeSet::from([EvidenceId::new(3)]), 750)?;

    assert_eq!(candidate.id(), MemoryCandidateId::new(1));
    assert_eq!(candidate.claim_id(), ClaimId::new(2));
    assert_eq!(
        candidate.evidence_ids(),
        &BTreeSet::from([EvidenceId::new(3)])
    );
    assert_eq!(candidate.confidence_milli(), 750);
    assert!(candidate.has_evidence());
    Ok(())
}
