use std::collections::BTreeMap;

use maestria_domain::{Claim, ClaimId, Memory, MemoryCandidate, MemoryCandidateId, MemoryId};

/// Pure read-only memory workflow analysis.
///
/// Memory state transitions (promotion, deprecation, contradiction,
/// supersession) are owned by the domain and always emit append-only domain
/// events (R40); this service only inspects state and must never mutate a
/// `Memory` or construct one.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryService;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictionCheck {
    pub new_candidate_id: MemoryCandidateId,
    pub existing_memory_id: MemoryId,
    pub reason: String,
}

impl MemoryService {
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
                memory.status == maestria_domain::MemoryStatus::Active
                    && memory.claim_id == candidate.claim_id
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
