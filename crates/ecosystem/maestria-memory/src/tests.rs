use crate::{ContradictionCheck, MemoryService};
use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{
    ArtifactId, Authority, Claim, ClaimId, ClaimStatus, EvidenceId, Memory, MemoryCandidate,
    MemoryCandidateId, MemoryId, MemoryStatus, SecurityMetadata, TrustZone,
};

fn evidence_ids(ids: &[u64]) -> BTreeSet<EvidenceId> {
    ids.iter().map(|id| EvidenceId::new(*id)).collect()
}

fn candidate(id: u64, claim_id: u64, evidence: &[u64]) -> MemoryCandidate {
    MemoryCandidate {
        id: MemoryCandidateId::new(id),
        claim_id: ClaimId::new(claim_id),
        evidence_ids: evidence_ids(evidence),
        confidence_milli: 900,
        security: SecurityMetadata {
            trust_zone: TrustZone::Verified,
            authority: Authority::User,
            ..SecurityMetadata::default()
        },
    }
}

fn memory(id: u64, candidate_id: u64, claim_id: u64, status: MemoryStatus) -> Memory {
    Memory {
        id: MemoryId::new(id),
        candidate_id: MemoryCandidateId::new(candidate_id),
        claim_id: ClaimId::new(claim_id),
        evidence_ids: evidence_ids(&[id]),
        status,
        security: SecurityMetadata::default(),
    }
}

fn claim(id: u64, text: &str) -> Claim {
    Claim {
        id: ClaimId::new(id),
        artifact_id: ArtifactId::new(1),
        text: text.to_string(),
        status: ClaimStatus::Verified,
        evidence_ids: evidence_ids(&[1]),
        security: SecurityMetadata::default(),
    }
}

#[test]
fn detect_contradictions_finds_same_claim_active_memories() {
    let candidate = candidate(10, 20, &[30]);
    let existing = BTreeMap::from([
        (MemoryId::new(1), memory(1, 101, 20, MemoryStatus::Active)),
        (
            MemoryId::new(2),
            memory(2, 102, 20, MemoryStatus::Deprecated),
        ),
        (MemoryId::new(3), memory(3, 103, 21, MemoryStatus::Active)),
    ]);
    let claims = BTreeMap::from([(ClaimId::new(20), claim(20, "The answer is 42"))]);

    let checks = MemoryService::detect_contradictions(&candidate, &existing, &claims);

    assert_eq!(
            checks,
            vec![ContradictionCheck {
                new_candidate_id: MemoryCandidateId::new(10),
                existing_memory_id: MemoryId::new(1),
                reason: "candidate claim 'The answer is 42' already has an active memory and requires contradiction review".to_string(),
            }]
        );
}

#[test]
fn detect_duplicates_finds_existing_candidate_with_same_claim() {
    let new_candidate = candidate(10, 20, &[30]);
    let existing = BTreeMap::from([
        (MemoryCandidateId::new(10), new_candidate.clone()),
        (MemoryCandidateId::new(11), candidate(11, 20, &[31])),
        (MemoryCandidateId::new(12), candidate(12, 21, &[32])),
    ]);

    let duplicates = MemoryService::detect_duplicates(&new_candidate, &existing);

    assert_eq!(duplicates, vec![MemoryCandidateId::new(11)]);
}

#[test]
fn review_queue_filters_already_promoted_candidates() {
    let candidates = BTreeMap::from([
        (MemoryCandidateId::new(10), candidate(10, 20, &[30])),
        (MemoryCandidateId::new(11), candidate(11, 21, &[31])),
        (MemoryCandidateId::new(12), candidate(12, 22, &[32])),
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
}
