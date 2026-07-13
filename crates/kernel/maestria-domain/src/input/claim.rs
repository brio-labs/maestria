use std::collections::BTreeSet;

use crate::types::*;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_create_claim(
        &mut self,
        input: CreateClaimInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.claims.contains_key(&input.claim_id) {
            return Err(DomainError::DuplicateId {
                kind: "claim",
                id: input.claim_id.value(),
            });
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut claim = Claim::new(input.claim_id, input.artifact_id, input.text.clone());
        let mut seen = BTreeSet::new();
        for evidence_id in &input.evidence_ids {
            if !seen.insert(*evidence_id) {
                return Err(DomainError::DuplicateId {
                    kind: "evidence_in_claim",
                    id: evidence_id.value(),
                });
            }
            let evidence = self
                .evidences
                .get(evidence_id)
                .ok_or(DomainError::MissingEvidence { id: *evidence_id })?;
            if evidence.artifact_id != input.artifact_id {
                return Err(DomainError::ArtifactMismatch {
                    expected: input.artifact_id,
                    actual: evidence.artifact_id,
                });
            }
            if let Some(existing_claim) = evidence.claim_id
                && existing_claim != input.claim_id
            {
                return Err(DomainError::DuplicateId {
                    kind: "evidence_claim",
                    id: evidence_id.value(),
                });
            }
            claim.evidence_ids.insert(*evidence_id);
        }
        for evidence_id in &input.evidence_ids {
            if let Some(evidence) = self.evidences.get_mut(evidence_id) {
                evidence.claim_id = Some(input.claim_id);
            }
        }

        self.claims.insert(input.claim_id, claim);
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.claim_ids.insert(input.claim_id);
        }

        Ok(self.emit_event(DomainEvent::ClaimCreated {
            claim_id: input.claim_id,
            artifact_id: input.artifact_id,
            text: input.text,
            evidence_ids: input.evidence_ids,
        }))
    }

    pub(super) fn handle_link_evidence_to_claim(
        &mut self,
        input: LinkEvidenceToClaimInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let claim = self
            .claims
            .get_mut(&input.claim_id)
            .ok_or(DomainError::MissingClaim { id: input.claim_id })?;
        let evidence =
            self.evidences
                .get(&input.evidence_id)
                .ok_or(DomainError::MissingEvidence {
                    id: input.evidence_id,
                })?;
        if evidence.artifact_id != claim.artifact_id {
            return Err(DomainError::ArtifactMismatch {
                expected: claim.artifact_id,
                actual: evidence.artifact_id,
            });
        }
        if let Some(existing_claim) = evidence.claim_id
            && existing_claim != input.claim_id
        {
            return Err(DomainError::DuplicateId {
                kind: "evidence_claim",
                id: input.evidence_id.value(),
            });
        }

        claim.evidence_ids.insert(input.evidence_id);
        if let Some(evidence) = self.evidences.get_mut(&input.evidence_id) {
            evidence.claim_id = Some(input.claim_id);
        }

        Ok(self.emit_event(DomainEvent::ClaimEvidenceLinked {
            claim_id: input.claim_id,
            evidence_id: input.evidence_id,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────

    pub(crate) fn apply_claim_created(
        &mut self,
        claim_id: ClaimId,
        artifact_id: ArtifactId,
        text: &str,
        evidence_ids: &[EvidenceId],
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        if self.claims.contains_key(&claim_id) {
            return Err(DomainError::DuplicateId {
                kind: "claim",
                id: claim_id.value(),
            });
        }
        let mut seen = BTreeSet::new();
        for evidence_id in evidence_ids {
            if !seen.insert(*evidence_id) {
                return Err(DomainError::DuplicateId {
                    kind: "evidence_in_claim",
                    id: evidence_id.value(),
                });
            }
            let evidence = self
                .evidences
                .get(evidence_id)
                .ok_or(DomainError::MissingEvidence { id: *evidence_id })?;
            if evidence.artifact_id != artifact_id {
                return Err(DomainError::ArtifactMismatch {
                    expected: artifact_id,
                    actual: evidence.artifact_id,
                });
            }
            if let Some(existing_claim) = evidence.claim_id
                && existing_claim != claim_id
            {
                return Err(DomainError::DuplicateId {
                    kind: "evidence_claim",
                    id: evidence_id.value(),
                });
            }
        }
        let mut claim = Claim::new(claim_id, artifact_id, text.to_string());
        claim.evidence_ids.extend(evidence_ids.iter().copied());
        for evidence_id in evidence_ids {
            if let Some(evidence) = self.evidences.get_mut(evidence_id) {
                evidence.claim_id = Some(claim_id);
            }
        }
        self.claims.insert(claim_id, claim);
        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.claim_ids.insert(claim_id);
        }
        Ok(())
    }

    pub(crate) fn apply_claim_validation_updated(
        &mut self,
        claim_id: ClaimId,
        status: &ClaimStatus,
    ) -> Result<(), DomainError> {
        let claim = self
            .claims
            .get_mut(&claim_id)
            .ok_or(DomainError::MissingClaim { id: claim_id })?;
        claim.status = status.clone();
        Ok(())
    }

    pub(crate) fn apply_claim_evidence_linked(
        &mut self,
        claim_id: ClaimId,
        evidence_id: EvidenceId,
    ) -> Result<(), DomainError> {
        let claim = self
            .claims
            .get_mut(&claim_id)
            .ok_or(DomainError::MissingClaim { id: claim_id })?;
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
        if let Some(existing_claim) = evidence.claim_id
            && existing_claim != claim_id
        {
            return Err(DomainError::DuplicateId {
                kind: "evidence_claim",
                id: evidence_id.value(),
            });
        }
        claim.evidence_ids.insert(evidence_id);
        if let Some(evidence) = self.evidences.get_mut(&evidence_id) {
            evidence.claim_id = Some(claim_id);
        }
        Ok(())
    }
}
