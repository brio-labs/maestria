use std::collections::BTreeSet;

use crate::types::*;

impl KernelState {
    pub(crate) fn apply_memory_candidate_created(
        &mut self,
        candidate_id: MemoryCandidateId,
        claim_id: ClaimId,
        evidence_ids: &BTreeSet<EvidenceId>,
        confidence_milli: u16,
    ) -> Result<(), DomainError> {
        if confidence_milli > 1000 {
            return Err(DomainError::InvalidConfidence {
                max: 1000,
                actual: confidence_milli,
            });
        }
        if self.memory_candidates.contains_key(&candidate_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory_candidate",
                id: candidate_id.value(),
            });
        }
        let claim = self
            .claims
            .get(&claim_id)
            .ok_or(DomainError::MissingClaim { id: claim_id })?;
        if evidence_ids.is_empty() {
            return Err(DomainError::EvidenceRequired {
                kind: "memory_candidate",
                id: candidate_id.value(),
            });
        }
        for evidence_id in evidence_ids {
            let evidence = self
                .evidences
                .get(evidence_id)
                .ok_or(DomainError::MissingEvidence { id: *evidence_id })?;
            if evidence.artifact_id != claim.artifact_id {
                return Err(DomainError::ArtifactMismatch {
                    expected: claim.artifact_id,
                    actual: evidence.artifact_id,
                });
            }
        }
        self.memory_candidates.insert(
            candidate_id,
            MemoryCandidate {
                id: candidate_id,
                claim_id,
                evidence_ids: evidence_ids.clone(),
                confidence_milli,
            },
        );
        Ok(())
    }

    pub(crate) fn apply_memory_promoted(
        &mut self,
        memory_id: MemoryId,
        candidate_id: MemoryCandidateId,
    ) -> Result<(), DomainError> {
        if self.memories.contains_key(&memory_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory",
                id: memory_id.value(),
            });
        }
        let candidate = self
            .memory_candidates
            .get(&candidate_id)
            .ok_or(DomainError::MissingMemoryCandidate { id: candidate_id })?;
        if candidate.evidence_ids.is_empty() {
            return Err(DomainError::MemoryCandidateIneligibleForPromotion {
                candidate_id: candidate.id,
                confidence_milli: candidate.confidence_milli,
                minimum_confidence_milli: MIN_PROMOTION_CONFIDENCE_MILLI,
                reason: "no evidence ids",
            });
        }
        if !candidate
            .evidence_ids
            .iter()
            .all(|evidence_id| self.evidences.contains_key(evidence_id))
        {
            return Err(DomainError::MemoryCandidateIneligibleForPromotion {
                candidate_id: candidate.id,
                confidence_milli: candidate.confidence_milli,
                minimum_confidence_milli: MIN_PROMOTION_CONFIDENCE_MILLI,
                reason: "missing evidence",
            });
        }
        if candidate.confidence_milli < MIN_PROMOTION_CONFIDENCE_MILLI {
            return Err(DomainError::MemoryCandidateIneligibleForPromotion {
                candidate_id: candidate.id,
                confidence_milli: candidate.confidence_milli,
                minimum_confidence_milli: MIN_PROMOTION_CONFIDENCE_MILLI,
                reason: "insufficient confidence",
            });
        }
        self.memories.insert(
            memory_id,
            Memory {
                id: memory_id,
                candidate_id,
                claim_id: candidate.claim_id,
                evidence_ids: candidate.evidence_ids.clone(),
                status: MemoryStatus::Active,
            },
        );
        Ok(())
    }

    pub(crate) fn apply_memory_contradicted(
        &mut self,
        memory_id: MemoryId,
        contradicting_candidate_id: MemoryCandidateId,
    ) -> Result<(), DomainError> {
        if !self
            .memory_candidates
            .contains_key(&contradicting_candidate_id)
        {
            return Err(DomainError::MissingMemoryCandidate {
                id: contradicting_candidate_id,
            });
        }
        let memory = self
            .memories
            .get_mut(&memory_id)
            .ok_or(DomainError::MissingMemory { id: memory_id })?;
        memory.status = MemoryStatus::Contradicted;
        Ok(())
    }

    pub(crate) fn apply_memory_deprecated(
        &mut self,
        memory_id: MemoryId,
    ) -> Result<(), DomainError> {
        let memory = self
            .memories
            .get_mut(&memory_id)
            .ok_or(DomainError::MissingMemory { id: memory_id })?;
        memory.status = MemoryStatus::Deprecated;
        Ok(())
    }

    pub(crate) fn apply_memory_superseded(
        &mut self,
        memory_id: MemoryId,
        by_memory_id: MemoryId,
    ) -> Result<(), DomainError> {
        if !self.memories.contains_key(&by_memory_id) {
            return Err(DomainError::MissingMemory { id: by_memory_id });
        }
        let memory = self
            .memories
            .get_mut(&memory_id)
            .ok_or(DomainError::MissingMemory { id: memory_id })?;
        memory.status = MemoryStatus::Superseded;
        Ok(())
    }
}
