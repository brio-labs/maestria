use std::sync::atomic::{AtomicU64, Ordering};

use crate::{IdAllocator, PortError};
use maestria_domain::{ApprovalId, ClaimId, MemoryCandidateId};

/// In-memory [`IdAllocator`] for tests and contract verification.
///
/// Two independent atomic counters guarantee per-namespace identity
/// without coupling.
#[derive(Debug, Default)]
pub struct InMemoryIdAllocator {
    claim_counter: AtomicU64,
    candidate_counter: AtomicU64,
    approval_counter: AtomicU64,
}

impl InMemoryIdAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the claim counter for tests that need predictable IDs.
    pub fn with_claim_seed(mut self, next: u64) -> Self {
        self.claim_counter = AtomicU64::new(next);
        self
    }
    /// Seed the candidate counter for tests that need predictable IDs.
    pub fn with_candidate_seed(mut self, next: u64) -> Self {
        self.candidate_counter = AtomicU64::new(next);
        self
    }

    /// Seed the approval counter for tests that need predictable IDs.
    pub fn with_approval_seed(mut self, next: u64) -> Self {
        self.approval_counter = AtomicU64::new(next);
        self
    }
}
impl IdAllocator for InMemoryIdAllocator {
    fn allocate_claim_id(&self) -> Result<ClaimId, PortError> {
        let id = self.claim_counter.fetch_add(1, Ordering::SeqCst);
        Ok(ClaimId::new(id + 1))
    }

    fn allocate_memory_candidate_id(&self) -> Result<MemoryCandidateId, PortError> {
        let id = self.candidate_counter.fetch_add(1, Ordering::SeqCst);
        Ok(MemoryCandidateId::new(id + 1))
    }

    fn allocate_approval_id(&self) -> Result<ApprovalId, PortError> {
        let id = self.approval_counter.fetch_add(1, Ordering::SeqCst);
        Ok(ApprovalId::new(id + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_independent_namespaces() -> Result<(), PortError> {
        let allocator = InMemoryIdAllocator::new();
        let c1 = allocator.allocate_claim_id()?;
        let mc1 = allocator.allocate_memory_candidate_id()?;
        let c2 = allocator.allocate_claim_id()?;

        assert_eq!(c1, ClaimId::new(1));
        assert_eq!(mc1, MemoryCandidateId::new(1));
        assert_eq!(c2, ClaimId::new(2));
        Ok(())
    }
    #[test]
    fn seeded_allocator_starts_at_given_value() -> Result<(), PortError> {
        let allocator = InMemoryIdAllocator::new()
            .with_claim_seed(5)
            .with_candidate_seed(10);

        assert_eq!(allocator.allocate_claim_id()?, ClaimId::new(6));
        assert_eq!(
            allocator.allocate_memory_candidate_id()?,
            MemoryCandidateId::new(11)
        );
        Ok(())
    }
}
