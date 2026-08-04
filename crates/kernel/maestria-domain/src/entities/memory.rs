use std::collections::BTreeSet;

use crate::ids::{ClaimId, EvidenceId, MemoryCandidateId, MemoryId};
use crate::security::SecurityMetadata;

/// Minimum candidate confidence (milli) required for memory promotion,
/// owned by the domain and reused by every promotion gate (R28).
pub const MIN_PROMOTION_CONFIDENCE_MILLI: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCandidate {
    id: MemoryCandidateId,
    claim_id: ClaimId,
    evidence_ids: BTreeSet<EvidenceId>,
    confidence_milli: u16,
    security: SecurityMetadata,
}

impl MemoryCandidate {
    /// Builds a candidate whose local promotion invariants are always valid.
    pub fn try_new(
        id: MemoryCandidateId,
        claim_id: ClaimId,
        evidence_ids: BTreeSet<EvidenceId>,
        confidence_milli: u16,
        security: SecurityMetadata,
    ) -> Result<Self, crate::DomainError> {
        if evidence_ids.is_empty() {
            return Err(crate::DomainError::EvidenceRequired {
                kind: "memory_candidate",
                id: id.value(),
            });
        }
        if confidence_milli > 1000 {
            return Err(crate::DomainError::InvalidConfidence {
                max: 1000,
                actual: confidence_milli,
            });
        }
        Ok(Self {
            id,
            claim_id,
            evidence_ids,
            confidence_milli,
            security,
        })
    }

    pub fn id(&self) -> MemoryCandidateId {
        self.id
    }

    pub fn claim_id(&self) -> ClaimId {
        self.claim_id
    }

    pub fn evidence_ids(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence_ids
    }

    pub fn confidence_milli(&self) -> u16 {
        self.confidence_milli
    }

    pub fn security(&self) -> &SecurityMetadata {
        &self.security
    }

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
