use std::collections::BTreeSet;

use crate::ids::{ClaimId, EvidenceId, MemoryCandidateId, MemoryId};
use crate::security::SecurityMetadata;

/// Minimum candidate confidence (milli) required for memory promotion,
/// owned by the domain and reused by every promotion gate (R28).
pub const MIN_PROMOTION_CONFIDENCE_MILLI: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub confidence_milli: u16,
    pub security: SecurityMetadata,
}

impl MemoryCandidate {
    pub fn has_evidence(&self) -> bool {
        !self.evidence_ids.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStatus {
    Active,
    Deprecated,
    Contradicted,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    pub id: MemoryId,
    pub candidate_id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub status: MemoryStatus,
    pub security: SecurityMetadata,
}
