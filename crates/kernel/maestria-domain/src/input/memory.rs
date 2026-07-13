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

    pub(super) fn handle_propose_memory_candidate(
        &mut self,
        input: ProposeMemoryCandidateInput,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let (artifact_id, evidence_ids) = self.validate_memory_proposal(&input)?;

        let mut claim = Claim::new(input.claim_id, artifact_id, input.text.clone());
        for &evidence_id in &evidence_ids {
            if let Some(evidence) = self.evidences.get_mut(&evidence_id) {
                evidence.claim_id = Some(input.claim_id);
            }
        }
        claim.evidence_ids = evidence_ids.clone();
        self.claims.insert(input.claim_id, claim);
        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.claim_ids.insert(input.claim_id);
        }
        let claim_created = self.emit_event(DomainEvent::ClaimCreated {
            claim_id: input.claim_id,
            artifact_id,
            text: input.text,
            evidence_ids: evidence_ids.iter().copied().collect(),
        });

        self.memory_candidates.insert(
            input.candidate_id,
            MemoryCandidate {
                id: input.candidate_id,
                claim_id: input.claim_id,
                evidence_ids: evidence_ids.clone(),
                confidence_milli: input.confidence_milli,
            },
        );
        let candidate_created = self.emit_event(DomainEvent::MemoryCandidateCreated {
            candidate_id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids,
            confidence_milli: input.confidence_milli,
        });
        Ok(vec![claim_created, candidate_created])
    }

    fn validate_memory_proposal(
        &self,
        input: &ProposeMemoryCandidateInput,
    ) -> Result<(ArtifactId, BTreeSet<EvidenceId>), DomainError> {
        if input.text.trim().is_empty() {
            return Err(DomainError::EmptyClaimText);
        }
        if input.confidence_milli > 1000 {
            return Err(DomainError::InvalidConfidence {
                max: 1000,
                actual: input.confidence_milli,
            });
        }
        if input.evidence_ids.is_empty() {
            return Err(DomainError::EvidenceRequired {
                kind: "memory_candidate",
                id: input.candidate_id.value(),
            });
        }
        if self.claims.contains_key(&input.claim_id) {
            return Err(DomainError::DuplicateId {
                kind: "claim",
                id: input.claim_id.value(),
            });
        }
        if self.memory_candidates.contains_key(&input.candidate_id) {
            return Err(DomainError::DuplicateId {
                kind: "memory_candidate",
                id: input.candidate_id.value(),
            });
        }

        let mut evidence_ids = BTreeSet::new();
        let mut artifact_id = None;
        for &evidence_id in &input.evidence_ids {
            if !evidence_ids.insert(evidence_id) {
                return Err(DomainError::DuplicateId {
                    kind: "evidence_in_claim",
                    id: evidence_id.value(),
                });
            }
            let evidence = self
                .evidences
                .get(&evidence_id)
                .ok_or(DomainError::MissingEvidence { id: evidence_id })?;
            if let Some(existing_claim) = evidence.claim_id
                && existing_claim != input.claim_id
            {
                return Err(DomainError::DuplicateId {
                    kind: "evidence_claim",
                    id: evidence_id.value(),
                });
            }
            match artifact_id {
                None => artifact_id = Some(evidence.artifact_id),
                Some(previous) if previous != evidence.artifact_id => {
                    return Err(DomainError::ArtifactMismatch {
                        expected: previous,
                        actual: evidence.artifact_id,
                    });
                }
                Some(_) => {}
            }
        }
        let artifact_id = artifact_id.ok_or(DomainError::EvidenceRequired {
            kind: "memory_candidate",
            id: input.candidate_id.value(),
        })?;
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        Ok((artifact_id, evidence_ids))
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
}
