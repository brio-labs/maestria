use std::collections::BTreeSet;

use crate::types::*;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_create_memory_candidate(
        &mut self,
        input: CreateMemoryCandidateInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if input.confidence_milli > 1000 {
            return Err(DomainError::InvalidConfidence {
                max: 1000,
                actual: input.confidence_milli,
            });
        }
        if self.memory_candidates.contains_key(&input.candidate_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory_candidate",
                id: input.candidate_id.value(),
            });
        }
        let claim = self
            .claims
            .get(&input.claim_id)
            .ok_or(DomainError::MissingClaim { id: input.claim_id })?;

        let mut evidence_ids = BTreeSet::new();
        for evidence_id in input.evidence_ids {
            let evidence = self
                .evidences
                .get(&evidence_id)
                .ok_or(DomainError::MissingEvidence { id: evidence_id })?;
            if evidence.artifact_id != claim.artifact_id {
                return Err(DomainError::ArtifactMismatch {
                    expected: claim.artifact_id,
                    actual: evidence.artifact_id,
                });
            }
            evidence_ids.insert(evidence_id);
        }
        if evidence_ids.is_empty() {
            return Err(DomainError::EvidenceRequired {
                kind: "memory_candidate",
                id: input.candidate_id.value(),
            });
        }

        let candidate = MemoryCandidate {
            id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids: evidence_ids.clone(),
            confidence_milli: input.confidence_milli,
        };
        self.memory_candidates.insert(input.candidate_id, candidate);
        Ok(self.emit_event(DomainEvent::MemoryCandidateCreated {
            candidate_id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids,
            confidence_milli: input.confidence_milli,
        }))
    }

    pub(super) fn handle_promote_memory(
        &mut self,
        input: PromoteMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let candidate = self.memory_candidates.get(&input.candidate_id).ok_or(
            DomainError::MissingMemoryCandidate {
                id: input.candidate_id,
            },
        )?;
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
        if self.memories.contains_key(&input.memory_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory",
                id: input.memory_id.value(),
            });
        }

        let memory = Memory {
            id: input.memory_id,
            candidate_id: input.candidate_id,
            claim_id: candidate.claim_id,
            evidence_ids: candidate.evidence_ids.clone(),
            status: MemoryStatus::Active,
        };
        self.memories.insert(input.memory_id, memory);

        Ok(self.emit_event(DomainEvent::MemoryPromoted {
            memory_id: input.memory_id,
            candidate_id: input.candidate_id,
        }))
    }

    pub(super) fn handle_contradict_memory(
        &mut self,
        input: ContradictMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self
            .memory_candidates
            .contains_key(&input.contradicting_candidate_id)
        {
            return Err(DomainError::MissingMemoryCandidate {
                id: input.contradicting_candidate_id,
            });
        }
        let memory = self
            .memories
            .get_mut(&input.memory_id)
            .ok_or(DomainError::MissingMemory {
                id: input.memory_id,
            })?;
        memory.status = MemoryStatus::Contradicted;

        Ok(self.emit_event(DomainEvent::MemoryContradicted {
            memory_id: input.memory_id,
            contradicting_candidate_id: input.contradicting_candidate_id,
        }))
    }

    pub(super) fn handle_deprecate_memory(
        &mut self,
        input: DeprecateMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let memory = self
            .memories
            .get_mut(&input.memory_id)
            .ok_or(DomainError::MissingMemory {
                id: input.memory_id,
            })?;
        memory.status = MemoryStatus::Deprecated;

        Ok(self.emit_event(DomainEvent::MemoryDeprecated {
            memory_id: input.memory_id,
        }))
    }

    pub(super) fn handle_supersede_memory(
        &mut self,
        input: SupersedeMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self.memories.contains_key(&input.by_memory_id) {
            return Err(DomainError::MissingMemory {
                id: input.by_memory_id,
            });
        }
        let memory = self
            .memories
            .get_mut(&input.memory_id)
            .ok_or(DomainError::MissingMemory {
                id: input.memory_id,
            })?;
        memory.status = MemoryStatus::Superseded;

        Ok(self.emit_event(DomainEvent::MemorySuperseded {
            memory_id: input.memory_id,
            by_memory_id: input.by_memory_id,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────

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
