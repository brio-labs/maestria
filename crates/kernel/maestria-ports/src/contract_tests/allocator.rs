//! Shared [`IdAllocator`] contract (Rule 25/27: concrete allocators keep
//! independent per-namespace identity; the shared suite proves the
//! namespace independence behavior).

use super::*;
use maestria_domain::{ApprovalId, MemoryCandidateId};

/// Namespaces allocate strictly increasing identities and advance
/// independently: allocating one namespace never moves another, and the
/// first allocation of a fresh namespace is unaffected by prior use of the
/// other namespaces.
pub fn assert_id_allocator_contract(
    allocator: &dyn IdAllocator,
) -> Result<(), Box<dyn std::error::Error>> {
    let claim_1 = allocator.allocate_claim_id()?;
    let claim_2 = allocator.allocate_claim_id()?;
    assert!(
        claim_2.value() > claim_1.value(),
        "claim ids must be strictly increasing"
    );

    let candidate_1 = allocator.allocate_memory_candidate_id()?;
    let candidate_2 = allocator.allocate_memory_candidate_id()?;
    assert!(
        candidate_2.value() > candidate_1.value(),
        "memory candidate ids must be strictly increasing"
    );

    let approval_1 = allocator.allocate_approval_id()?;
    let approval_2 = allocator.allocate_approval_id()?;
    assert!(
        approval_2.value() > approval_1.value(),
        "approval ids must be strictly increasing"
    );

    // Independent namespaces: claim allocations between candidate and
    // approval allocations must not disturb either counter.
    let claim_3 = allocator.allocate_claim_id()?;
    assert!(claim_3.value() > claim_2.value());
    assert_eq!(
        allocator.allocate_memory_candidate_id()?,
        MemoryCandidateId::new(candidate_2.value().saturating_add(1)),
        "claim allocation must not advance the candidate namespace"
    );
    assert_eq!(
        allocator.allocate_approval_id()?,
        ApprovalId::new(approval_2.value().saturating_add(1)),
        "claim allocation must not advance the approval namespace"
    );
    Ok(())
}
