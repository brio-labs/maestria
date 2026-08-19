use std::collections::BTreeMap;

use maestria_domain::{Memory, MemoryCandidate, MemoryCandidateId, MemoryId};

/// Lists candidate ids that have not already been promoted into a memory.
///
/// Pure read-only memory workflow analysis. Memory state transitions
/// (promotion, deprecation, contradiction, supersession) are owned by the
/// domain and always emit append-only domain events (R40); this function
/// only inspects state and must never mutate a `Memory` or construct one.
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
