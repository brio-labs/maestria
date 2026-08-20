use crate::provenance::evidence_id_for;
use crate::security::SecurityMetadata;
use crate::types::*;

pub(crate) struct ApplyEvidenceRecordedArgs<'a> {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub kind: &'a EvidenceKind,
    pub excerpt: &'a str,
    pub observed_at: LogicalTick,
    pub security: &'a SecurityMetadata,
}

impl KernelState {
    // ── Deterministic evidence helpers ────────────────────────────

    /// Returns the chunk whose deterministic `evidence_id_for` mapping
    /// produces `evidence_id`, if any.
    fn deterministic_chunk_for(&self, evidence_id: EvidenceId) -> Option<&Chunk> {
        self.chunks
            .values()
            .find(|chunk| evidence_id_for(chunk.artifact_id, chunk.order) == evidence_id)
    }

    fn is_deterministic_evidence_id(&self, evidence_id: EvidenceId) -> bool {
        self.deterministic_chunk_for(evidence_id).is_some()
    }

    /// Validates that incoming evidence at a deterministic ID is a
    /// source-backed `FileSpan` with a snapshot whose content hash matches
    /// the artifact's recorded content hash, and whose artifact owner is
    /// the same chunk owner.
    fn validate_deterministic_evidence(
        &self,
        input: &RecordEvidenceInput,
    ) -> Result<(), DomainError> {
        let chunk = match self.deterministic_chunk_for(input.evidence_id) {
            Some(c) => c,
            None => {
                return Err(DomainError::MalformedDeterministicEvidence {
                    evidence_id: input.evidence_id,
                    reason: "evidence ID does not match any chunk",
                });
            }
        };
        if input.artifact_id != chunk.artifact_id {
            return Err(DomainError::MalformedDeterministicEvidence {
                evidence_id: input.evidence_id,
                reason: "artifact_id does not match chunk owner",
            });
        }
        let expected_hash = self
            .artifacts
            .get(&chunk.artifact_id)
            .and_then(|artifact| artifact.content_hash.as_ref());
        match &input.kind {
            EvidenceKind::FileSpan { snapshot, .. }
            | EvidenceKind::PdfSpan { snapshot, .. }
            | EvidenceKind::PdfRegion { snapshot, .. }
                if expected_hash == Some(snapshot.content_hash()) =>
            {
                Ok(())
            }
            EvidenceKind::FileSpan { .. }
            | EvidenceKind::PdfSpan { .. }
            | EvidenceKind::PdfRegion { .. } => Err(DomainError::MalformedDeterministicEvidence {
                evidence_id: input.evidence_id,
                reason: "snapshot content hash does not match artifact content_hash",
            }),
            _ => Err(DomainError::MalformedDeterministicEvidence {
                evidence_id: input.evidence_id,
                reason: "evidence must be a source-backed span with a snapshot",
            }),
        }
    }

    /// Returns `true` when every chunk of `artifact_id` has a corresponding
    /// deterministic `FileSpan` whose snapshot hash matches the artifact.
    /// Zero-chunk artifacts trivially satisfy the check.
    pub(crate) fn evidence_complete_for(&self, artifact_id: ArtifactId) -> bool {
        let Some(artifact) = self.artifacts.get(&artifact_id) else {
            return false;
        };
        let Some(expected_hash) = &artifact.content_hash else {
            return false;
        };
        for chunk in self.chunks.values() {
            if chunk.artifact_id != artifact_id {
                continue;
            }
            let expected_id = evidence_id_for(chunk.artifact_id, chunk.order);
            let ev = match self.evidences.get(&expected_id) {
                Some(ev) => ev,
                None => return false,
            };
            if ev.artifact_id != artifact_id {
                return false;
            }
            match &ev.kind {
                EvidenceKind::FileSpan { snapshot, .. }
                | EvidenceKind::PdfSpan { snapshot, .. }
                | EvidenceKind::PdfRegion { snapshot, .. }
                    if snapshot.content_hash() == expected_hash =>
                {
                    continue;
                }
                _ => return false,
            }
        }
        true
    }

    // ── Handler ──────────────────────────────────────────────────

    pub(super) fn handle_record_evidence(
        &mut self,
        input: RecordEvidenceInput,
    ) -> Result<Option<DomainEventEnvelope>, DomainError> {
        let is_deterministic = self.is_deterministic_evidence_id(input.evidence_id);

        if is_deterministic {
            self.validate_deterministic_evidence(&input)?;
        }

        if let Some(existing) = self.evidences.get(&input.evidence_id) {
            if existing.artifact_id == input.artifact_id
                && existing.claim_id == input.claim_id
                && existing.kind == input.kind
                && existing.excerpt == input.excerpt
                && existing.observed_at == input.observed_at
            {
                return Ok(None);
            }
            return Err(DomainError::DuplicateEvidence {
                id: input.evidence_id,
            });
        }

        // ── Validate incoming artifact / claim *before* any mutation ──
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if let Some(claim_id) = input.claim_id {
            let claim = self
                .claims
                .get(&claim_id)
                .ok_or(DomainError::MissingClaim { id: claim_id })?;
            if claim.artifact_id != input.artifact_id {
                return Err(DomainError::ArtifactMismatch {
                    expected: input.artifact_id,
                    actual: claim.artifact_id,
                });
            }
        }

        let kind = input.kind.clone();
        let mut security = SecurityMetadata::from_optional(input.security);
        if let Some(artifact) = self.artifacts.get(&input.artifact_id) {
            security = security.taint_from(&artifact.security);
        }
        if let Some(claim_id) = input.claim_id
            && let Some(claim) = self.claims.get(&claim_id)
        {
            security = security.taint_from(&claim.security);
        }
        self.evidences.insert(
            input.evidence_id,
            Evidence::new(
                input.evidence_id,
                input.artifact_id,
                input.claim_id,
                kind.clone(),
                input.excerpt.clone(),
                input.observed_at,
                security.clone(),
            ),
        );

        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.evidence_ids.insert(input.evidence_id);
        }
        if let Some(claim_id) = input.claim_id
            && let Some(claim) = self.claims.get_mut(&claim_id)
        {
            claim.evidence_ids.insert(input.evidence_id);
        }

        Ok(Some(self.emit_event(DomainEvent::EvidenceRecorded {
            evidence_id: input.evidence_id,
            artifact_id: input.artifact_id,
            claim_id: input.claim_id,
            kind,
            excerpt: input.excerpt,
            observed_at: input.observed_at,
            security,
        })))
    }

    // ── Replay apply ─────────────────────────────────────────────
    pub(crate) fn apply_evidence_recorded(
        &mut self,
        args: ApplyEvidenceRecordedArgs<'_>,
    ) -> Result<(), DomainError> {
        let ApplyEvidenceRecordedArgs {
            evidence_id,
            artifact_id,
            claim_id,
            kind,
            excerpt,
            observed_at,
            security,
        } = args;
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        if let Some(chunk) = self.deterministic_chunk_for(evidence_id) {
            if artifact_id != chunk.artifact_id {
                return Err(DomainError::MalformedDeterministicEvidence {
                    evidence_id,
                    reason: "artifact_id does not match chunk owner",
                });
            }
            let expected_hash = self
                .artifacts
                .get(&chunk.artifact_id)
                .and_then(|artifact| artifact.content_hash.as_ref());
            let valid = match kind {
                EvidenceKind::FileSpan { snapshot, .. }
                | EvidenceKind::PdfSpan { snapshot, .. }
                | EvidenceKind::PdfRegion { snapshot, .. } => {
                    expected_hash == Some(snapshot.content_hash())
                }
                _ => false,
            };
            if !valid {
                return Err(DomainError::MalformedDeterministicEvidence {
                    evidence_id,
                    reason: "deterministic evidence requires a matching snapshot",
                });
            }
        }
        if let Some(existing) = self.evidences.get(&evidence_id)
            && (existing.artifact_id != artifact_id
                || existing.claim_id != claim_id
                || existing.kind != *kind
                || existing.excerpt != excerpt
                || existing.observed_at != observed_at)
        {
            return Err(DomainError::DuplicateEvidence { id: evidence_id });
        }

        // Validate the incoming claim before any mutation.
        if let Some(claim_id) = claim_id {
            let claim = self
                .claims
                .get(&claim_id)
                .ok_or(DomainError::MissingClaim { id: claim_id })?;
            if claim.artifact_id != artifact_id {
                return Err(DomainError::ArtifactMismatch {
                    expected: artifact_id,
                    actual: claim.artifact_id,
                });
            }
        }

        // Insert new evidence and reverse links.
        self.evidences.insert(
            evidence_id,
            Evidence::new(
                evidence_id,
                artifact_id,
                claim_id,
                kind.clone(),
                excerpt.to_string(),
                observed_at,
                security.clone(),
            ),
        );

        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.evidence_ids.insert(evidence_id);
        }
        if let Some(cid) = claim_id
            && let Some(claim) = self.claims.get_mut(&cid)
        {
            claim.evidence_ids.insert(evidence_id);
        }
        Ok(())
    }
}
