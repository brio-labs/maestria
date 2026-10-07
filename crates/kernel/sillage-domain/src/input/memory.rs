use std::collections::BTreeSet;
use std::sync::Arc;

use crate::security::SecurityMetadata;
use crate::types::*;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_create_memory_candidate(
        &mut self,
        input: CreateMemoryCandidateInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        // The shared apply owns candidate validation; the event carries the
        // live-derived security so replay re-taints it against current state.
        let mut security = SecurityMetadata::from_optional(input.security);
        if let Some(claim) = self.claims.get(&input.claim_id) {
            security = security.taint_from(&claim.security);
        }
        let mut evidence_ids = BTreeSet::new();
        for evidence_id in input.evidence_ids {
            if let Some(evidence) = self.evidences.get(&evidence_id) {
                security = security.taint_from(&evidence.security);
            }
            evidence_ids.insert(evidence_id);
        }
        self.apply_memory_candidate_created(
            input.candidate_id,
            input.claim_id,
            &evidence_ids,
            input.confidence_milli,
            &security,
        )?;
        Ok(self.emit_event(DomainEvent::MemoryCandidateCreated {
            candidate_id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids,
            confidence_milli: input.confidence_milli,
            security,
        }))
    }

    pub(super) fn handle_propose_memory_candidate(
        &mut self,
        input: ProposeMemoryCandidateInput,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let (artifact_id, evidence_ids) = self.validate_memory_proposal(&input)?;

        let mut security = SecurityMetadata::from_optional(input.security);
        if let Some(artifact) = self.artifacts.get(&artifact_id) {
            security = security.taint_from(&artifact.security);
        }
        for ev_id in &evidence_ids {
            if let Some(ev) = self.evidences.get(ev_id) {
                security = security.taint_from(&ev.security);
            }
        }

        let mut claim = Claim::new(
            input.claim_id,
            artifact_id,
            input.text.clone(),
            security.clone(),
        );
        claim.evidence_ids = evidence_ids.clone();
        Arc::make_mut(&mut self.claims).insert(input.claim_id, claim);
        for ev_id in &evidence_ids {
            if let Some(ev) = Arc::make_mut(&mut self.evidences).get_mut(ev_id) {
                ev.claim_id = Some(input.claim_id);
            }
        }
        if let Some(artifact) = Arc::make_mut(&mut self.artifacts).get_mut(&artifact_id) {
            artifact.claim_ids.insert(input.claim_id);
        }
        let claim_created = self.emit_event(DomainEvent::ClaimCreated {
            claim_id: input.claim_id,
            artifact_id,
            text: input.text,
            evidence_ids: evidence_ids.iter().copied().collect(),
            security: security.clone(),
        });

        Arc::make_mut(&mut self.memory_candidates).insert(
            input.candidate_id,
            MemoryCandidate::try_new(
                input.candidate_id,
                input.claim_id,
                evidence_ids.clone(),
                input.confidence_milli,
                security.clone(),
            )?,
        );
        let candidate_created = self.emit_event(DomainEvent::MemoryCandidateCreated {
            candidate_id: input.candidate_id,
            claim_id: input.claim_id,
            evidence_ids,
            confidence_milli: input.confidence_milli,
            security,
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
            return Err(DomainError::MemoryCandidateRequiresEvidence {
                id: input.candidate_id,
            });
        }
        if self.claims.contains_key(&input.claim_id) {
            return Err(DomainError::DuplicateClaim { id: input.claim_id });
        }
        if self.memory_candidates.contains_key(&input.candidate_id) {
            return Err(DomainError::DuplicateMemoryCandidate {
                id: input.candidate_id,
            });
        }

        let mut evidence_ids = BTreeSet::new();
        let mut artifact_id = None;
        for &evidence_id in &input.evidence_ids {
            if !evidence_ids.insert(evidence_id) {
                return Err(DomainError::DuplicateEvidenceInClaim { id: evidence_id });
            }
            let evidence = self
                .evidences
                .get(&evidence_id)
                .ok_or(DomainError::MissingEvidence { id: evidence_id })?;
            if let Some(existing_claim) = evidence.claim_id
                && existing_claim != input.claim_id
            {
                return Err(DomainError::DuplicateEvidenceClaim { id: evidence_id });
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
        let artifact_id = artifact_id.ok_or(DomainError::MemoryCandidateRequiresEvidence {
            id: input.candidate_id,
        })?;
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        Ok((artifact_id, evidence_ids))
    }

    pub(crate) fn current_memory_security(&self, candidate: &MemoryCandidate) -> SecurityMetadata {
        let mut security = candidate.security().clone();
        if let Some(claim) = self.claims.get(&candidate.claim_id()) {
            security = security.taint_from(&claim.security);
        }
        for evidence_id in candidate.evidence_ids() {
            if let Some(evidence) = self.evidences.get(evidence_id) {
                security = security.taint_from(&evidence.security);
            }
        }
        security
    }

    pub(super) fn handle_promote_memory(
        &mut self,
        input: PromoteMemoryInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        // The shared apply owns the promotion gates; the event carries the
        // live-derived security so replay re-taints it against current state.
        let candidate = self.memory_candidates.get(&input.candidate_id).ok_or(
            DomainError::MissingMemoryCandidate {
                id: input.candidate_id,
            },
        )?;
        let security = self.current_memory_security(candidate);
        self.apply_memory_promoted(input.memory_id, input.candidate_id, &security)?;
        Ok(self.emit_event(DomainEvent::MemoryPromoted {
            memory_id: input.memory_id,
            candidate_id: input.candidate_id,
            security,
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
        let memory = Arc::make_mut(&mut self.memories)
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
        let memory = Arc::make_mut(&mut self.memories)
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
        if input.by_memory_id == input.memory_id {
            return Err(DomainError::MemorySupersedesItself {
                memory_id: input.memory_id,
            });
        }
        if !self.memories.contains_key(&input.by_memory_id) {
            return Err(DomainError::MissingMemory {
                id: input.by_memory_id,
            });
        }
        let memory = Arc::make_mut(&mut self.memories)
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
