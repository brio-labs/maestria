use crate::{ContradictionCheck, MemoryService, PromoteMemoryInput, PromoteMemoryOutput};
use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{
    ArtifactId, Authority, Claim, ClaimId, ClaimStatus, EvidenceId, Memory, MemoryCandidate,
    MemoryCandidateId, MemoryId, MemoryStatus, SecurityMetadata, TrustZone,
};
use maestria_governance::{
    DefaultMemoryPromotionGate, MemoryPromotionDecision, MemoryPromotionGate,
    MemoryPromotionRequest,
};

#[derive(Debug)]
struct FixedGate {
    decision: MemoryPromotionDecision,
}

impl MemoryPromotionGate for FixedGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision {
        assert_eq!(request.candidate.id, MemoryCandidateId::new(10));
        assert!(request.user_approved);
        self.decision.clone()
    }
}

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
fn promote_returns_active_memory_when_gate_allows() {
    let candidate = candidate(10, 20, &[30]);
    let input = PromoteMemoryInput {
        memory_id: MemoryId::new(40),
        candidate: candidate.clone(),
        user_approved: true,
    };

    let output = MemoryService::promote(input, &DefaultMemoryPromotionGate);

    assert_eq!(
        output,
        PromoteMemoryOutput::Promoted(Memory {
            id: MemoryId::new(40),
            candidate_id: candidate.id,
            claim_id: candidate.claim_id,
            evidence_ids: candidate.evidence_ids,
            status: MemoryStatus::Active,
            security: candidate.security.clone(),
        })
    );
}

#[test]
fn promote_requires_evidence_when_gate_requires_evidence() {
    let input = PromoteMemoryInput {
        memory_id: MemoryId::new(40),
        candidate: candidate(10, 20, &[]),
        user_approved: true,
    };

    let output = MemoryService::promote(input, &DefaultMemoryPromotionGate);

    assert!(matches!(
        output,
        PromoteMemoryOutput::RequiresEvidence { reason } if reason.contains("evidence")
    ));
}

#[test]
fn promote_refuses_evidence_less_candidate_even_when_gate_allows() {
    let input = PromoteMemoryInput {
        memory_id: MemoryId::new(40),
        candidate: candidate(10, 20, &[]),
        user_approved: true,
    };
    let gate = FixedGate {
        decision: MemoryPromotionDecision::Promote,
    };

    let output = MemoryService::promote(input, &gate);

    assert!(matches!(
        output,
        PromoteMemoryOutput::RequiresEvidence { reason } if reason.contains("evidence")
    ));
}

#[test]
fn promote_requires_review_without_user_approval() {
    let input = PromoteMemoryInput {
        memory_id: MemoryId::new(40),
        candidate: candidate(10, 20, &[30]),
        user_approved: false,
    };

    let output = MemoryService::promote(input, &DefaultMemoryPromotionGate);

    assert!(matches!(
        output,
        PromoteMemoryOutput::RequiresReview { reason } if reason.contains("approval")
    ));
}

#[test]
fn promotion_delegates_to_memory_promotion_gate() {
    let input = PromoteMemoryInput {
        memory_id: MemoryId::new(40),
        candidate: candidate(10, 20, &[30]),
        user_approved: true,
    };
    let gate = FixedGate {
        decision: MemoryPromotionDecision::Deny {
            reason: "test gate denial".to_string(),
        },
    };

    let output = MemoryService::promote(input, &gate);

    assert_eq!(
        output,
        PromoteMemoryOutput::Denied {
            reason: "test gate denial".to_string(),
        }
    );
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

#[test]
fn deprecate_marks_memory_deprecated() {
    let mut memory = memory(1, 10, 20, MemoryStatus::Active);

    let updated = MemoryService::deprecate(MemoryId::new(1), &mut memory);

    assert_eq!(updated.status, MemoryStatus::Deprecated);
    assert_eq!(memory.status, MemoryStatus::Deprecated);
}

#[test]
fn mark_contradicted_marks_memory_contradicted() {
    let mut memory = memory(1, 10, 20, MemoryStatus::Active);

    let updated = MemoryService::mark_contradicted(MemoryId::new(1), &mut memory);

    assert_eq!(updated.status, MemoryStatus::Contradicted);
    assert_eq!(memory.status, MemoryStatus::Contradicted);
}

#[test]
fn supersede_marks_memory_superseded() {
    let mut memory = memory(1, 10, 20, MemoryStatus::Active);

    let updated = MemoryService::supersede(MemoryId::new(1), &mut memory);

    assert_eq!(updated.status, MemoryStatus::Superseded);
    assert_eq!(memory.status, MemoryStatus::Superseded);
}
