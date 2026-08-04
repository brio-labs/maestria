use crate::MemoryService;
use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{
    Authority, EvidenceId, Memory, MemoryCandidate, MemoryCandidateId, MemoryId, MemoryStatus,
    SecurityMetadata, TrustZone,
};

fn evidence_ids(ids: &[u64]) -> BTreeSet<EvidenceId> {
    ids.iter().map(|id| EvidenceId::new(*id)).collect()
}

fn candidate(
    id: u64,
    claim_id: u64,
    evidence: &[u64],
) -> Result<MemoryCandidate, maestria_domain::DomainError> {
    MemoryCandidate::try_new(
        MemoryCandidateId::new(id),
        maestria_domain::ClaimId::new(claim_id),
        evidence_ids(evidence),
        900,
        SecurityMetadata {
            trust_zone: TrustZone::Verified,
            authority: Authority::User,
            ..SecurityMetadata::default()
        },
    )
}

fn memory(id: u64, candidate_id: u64, claim_id: u64, status: MemoryStatus) -> Memory {
    Memory {
        id: MemoryId::new(id),
        candidate_id: MemoryCandidateId::new(candidate_id),
        claim_id: maestria_domain::ClaimId::new(claim_id),
        evidence_ids: evidence_ids(&[id]),
        status,
        security: SecurityMetadata::default(),
    }
}

#[test]
fn review_queue_filters_already_promoted_candidates() -> Result<(), Box<dyn std::error::Error>> {
    let candidates = BTreeMap::from([
        (MemoryCandidateId::new(10), candidate(10, 20, &[30])?),
        (MemoryCandidateId::new(11), candidate(11, 21, &[31])?),
        (MemoryCandidateId::new(12), candidate(12, 22, &[32])?),
    ]);
    let existing = BTreeMap::from([
        (MemoryId::new(1), memory(1, 10, 20, MemoryStatus::Active)),
        (
            MemoryId::new(2),
            memory(2, 12, 22, MemoryStatus::Superseded),
        ),
    ]);

    let queue = MemoryService::review_queue(&candidates, &existing);

    assert_eq!(queue, vec![MemoryCandidateId::new(11)]);
    Ok(())
}
