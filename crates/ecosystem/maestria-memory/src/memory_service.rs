use std::collections::BTreeMap;

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
        // Promoted memories must point back to evidence: an evidence-less
        // candidate cannot become an active memory regardless of gate policy.
        if !input.candidate.has_evidence() {
            return PromoteMemoryOutput::RequiresEvidence {
                reason:
                    "candidate carries no evidence; promoted memories must point back to evidence"
                        .to_string(),
            };
        }
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
