#![forbid(unsafe_code)]

//! Pure memory workflow orchestration for Maestria.

use std::collections::BTreeMap;

#[cfg(test)]
use maestria_domain::{Authority, SecurityMetadata, TrustZone};
use maestria_domain::{
    Claim, ClaimId, Memory, MemoryCandidate, MemoryCandidateId, MemoryId, MemoryStatus,
};
use maestria_governance::{MemoryPromotionDecision, MemoryPromotionGate, MemoryPromotionRequest};

/// Pure orchestration of memory workflows.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryService;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteMemoryInput {
    pub memory_id: MemoryId,
    pub candidate: MemoryCandidate,
    pub user_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromoteMemoryOutput {
    Promoted(Memory),
    RequiresEvidence { reason: String },
    RequiresReview { reason: String },
    Denied { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictionCheck {
    pub new_candidate_id: MemoryCandidateId,
    pub existing_memory_id: MemoryId,
    pub reason: String,
}

impl MemoryService {
    /// Evaluates a candidate against the supplied governance gate and promotes it
    /// to an active memory only when policy allows promotion.
    pub fn promote(
        input: PromoteMemoryInput,
        gate: &dyn MemoryPromotionGate,
    ) -> PromoteMemoryOutput {
        let request = MemoryPromotionRequest {
            candidate: input.candidate.clone(),
            user_approved: input.user_approved,
        };

        match gate.evaluate(&request) {
            MemoryPromotionDecision::Promote => PromoteMemoryOutput::Promoted(Memory {
                id: input.memory_id,
                candidate_id: input.candidate.id,
                claim_id: input.candidate.claim_id,
                evidence_ids: input.candidate.evidence_ids,
                status: MemoryStatus::Active,
                security: input.candidate.security,
            }),
            MemoryPromotionDecision::RequireEvidence { reason } => {
                PromoteMemoryOutput::RequiresEvidence { reason }
            }
            MemoryPromotionDecision::RequireReview { reason } => {
                PromoteMemoryOutput::RequiresReview { reason }
            }
            MemoryPromotionDecision::Deny { reason } => PromoteMemoryOutput::Denied { reason },
        }
    }

    /// Detects active memories carrying the same claim as a new candidate.
    ///
    /// The domain models explicit contradiction relations elsewhere; at the memory
    /// workflow level, a new candidate for a claim that is already represented by
    /// an active memory must be surfaced for review instead of silently replacing
    /// the existing memory.
    pub fn detect_contradictions(
        candidate: &MemoryCandidate,
        existing: &BTreeMap<MemoryId, Memory>,
        claims: &BTreeMap<ClaimId, Claim>,
    ) -> Vec<ContradictionCheck> {
        existing
            .iter()
            .filter(|(_, memory)| {
                memory.status == MemoryStatus::Active && memory.claim_id == candidate.claim_id
            })
            .map(|(memory_id, _)| ContradictionCheck {
                new_candidate_id: candidate.id,
                existing_memory_id: *memory_id,
                reason: contradiction_reason(candidate.claim_id, claims),
            })
            .collect()
    }

    /// Finds existing candidates that target the same claim as the new candidate.
    pub fn detect_duplicates(
        candidate: &MemoryCandidate,
        existing: &BTreeMap<MemoryCandidateId, MemoryCandidate>,
    ) -> Vec<MemoryCandidateId> {
        existing
            .iter()
            .filter(|(candidate_id, existing_candidate)| {
                **candidate_id != candidate.id && existing_candidate.claim_id == candidate.claim_id
            })
            .map(|(candidate_id, _)| *candidate_id)
            .collect()
    }

    /// Lists candidate ids that have not already been promoted into a memory.
    pub fn review_queue(
        candidates: &BTreeMap<MemoryCandidateId, MemoryCandidate>,
        existing: &BTreeMap<MemoryId, Memory>,
    ) -> Vec<MemoryCandidateId> {
        candidates
            .keys()
            .filter(|candidate_id| {
                !existing
                    .values()
                    .any(|memory| memory.candidate_id == **candidate_id)
            })
            .copied()
            .collect()
    }

    /// Marks a memory as deprecated and returns the updated value.
    pub fn deprecate(_memory_id: MemoryId, memory: &mut Memory) -> Memory {
        memory.status = MemoryStatus::Deprecated;
        memory.clone()
    }

    /// Marks a memory as contradicted and returns the updated value.
    pub fn mark_contradicted(_memory_id: MemoryId, memory: &mut Memory) -> Memory {
        memory.status = MemoryStatus::Contradicted;
        memory.clone()
    }

    /// Marks a memory as superseded and returns the updated value.
    pub fn supersede(_memory_id: MemoryId, memory: &mut Memory) -> Memory {
        memory.status = MemoryStatus::Superseded;
        memory.clone()
    }
}

fn contradiction_reason(claim_id: ClaimId, claims: &BTreeMap<ClaimId, Claim>) -> String {
    if let Some(claim) = claims.get(&claim_id) {
        format!(
            "candidate claim '{}' already has an active memory and requires contradiction review",
            claim.text
        )
    } else {
        format!(
            "candidate claim {claim_id} already has an active memory and requires contradiction review"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use maestria_domain::{ArtifactId, ClaimStatus, EvidenceId};
    use maestria_governance::DefaultMemoryPromotionGate;

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
}
