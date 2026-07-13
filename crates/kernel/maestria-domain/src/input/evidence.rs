use crate::provenance::evidence_id_for;
use crate::types::*;

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
    /// source-backed `FileSpan` with a `Some` snapshot and a
    /// `content_hash` matching the artifact's recorded content hash, and
    /// `input.artifact_id` matching the chunk's owning artifact.
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
            .and_then(|artifact| artifact.content_hash.as_deref());
        match &input.kind {
            EvidenceKind::FileSpan {
                content_hash: _content_hash,
                snapshot: Some(_),
                ..
            } if expected_hash == Some(_content_hash.as_str()) => Ok(()),
            EvidenceKind::FileSpan {
                snapshot: Some(_), ..
            } => Err(DomainError::MalformedDeterministicEvidence {
                evidence_id: input.evidence_id,
                reason: "content_hash does not match artifact content_hash",
            }),
            EvidenceKind::PdfSpan { .. } => Ok(()),
            _ => Err(DomainError::MalformedDeterministicEvidence {
                evidence_id: input.evidence_id,
                reason: "evidence must be a source-backed FileSpan with a snapshot",
            }),
        }
    }

    /// Returns `true` when `ev` is a valid deterministic evidence
    /// record: its ID matches a chunk's deterministic mapping, its
    /// `artifact_id` matches the chunk's owner, its kind is
    /// `FileSpan` with a `Some` snapshot, and its `content_hash`
    /// equals the artifact's recorded content hash.
    fn is_valid_deterministic_evidence(&self, ev: &Evidence) -> bool {
        let Some(chunk) = self.deterministic_chunk_for(ev.id) else {
            return false;
        };
        if ev.artifact_id != chunk.artifact_id {
            return false;
        }
        let expected_hash = match self
            .artifacts
            .get(&chunk.artifact_id)
            .and_then(|a| a.content_hash.as_deref())
        {
            Some(h) => h,
            None => return false,
        };
        matches!(
            &ev.kind,
            EvidenceKind::FileSpan {
                content_hash,
                snapshot: Some(_),
                ..
            } if content_hash == expected_hash
        ) || matches!(&ev.kind, EvidenceKind::PdfSpan { .. })
    }

    /// Returns `true` when the record represents a valid deterministic
    /// source-evidence record: its ID maps to a known chunk, its
    /// `artifact_id` matches the chunk owner, its kind is `FileSpan`
    /// with a `Some` snapshot, and its `content_hash` equals the
    /// artifact's recorded content hash.
    pub(super) fn is_valid_deterministic_record(
        &self,
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        kind: &EvidenceKind,
    ) -> bool {
        let Some(chunk) = self
            .chunks
            .values()
            .find(|chunk| evidence_id_for(chunk.artifact_id, chunk.order) == evidence_id)
        else {
            return false;
        };
        if artifact_id != chunk.artifact_id {
            return false;
        }
        let Some(expected_hash) = self
            .artifacts
            .get(&chunk.artifact_id)
            .and_then(|a| a.content_hash.as_deref())
        else {
            return false;
        };
        matches!(
            kind,
            EvidenceKind::FileSpan {
                content_hash,
                snapshot: Some(_),
                ..
            } if content_hash == expected_hash
        )
    }

    /// Returns `true` when every chunk of `artifact_id` has a corresponding
    /// evidence record whose ID matches the deterministic `evidence_id_for`
    /// mapping, whose `artifact_id` field matches, whose kind is
    /// `EvidenceKind::FileSpan` with a `Some` snapshot (source-backed), and
    /// whose `content_hash` equals the artifact's recorded content hash.
    /// Zero-chunk artifacts trivially satisfy the check.
    pub(crate) fn evidence_complete_for(&self, artifact_id: ArtifactId) -> bool {
        let Some(artifact) = self.artifacts.get(&artifact_id) else {
            return false;
        };
        let Some(ref expected_hash) = artifact.content_hash else {
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
                EvidenceKind::FileSpan {
                    content_hash,
                    snapshot: Some(_snapshot),
                    ..
                } if content_hash == expected_hash => continue,
                EvidenceKind::PdfSpan { .. } => continue,
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

        // Determine what to do about an existing record at this ID.
        // For deterministic IDs, a malformed existing record is eligible
        // for replacement once all incoming validation passes.
        let should_replace: Option<(ArtifactId, Option<ClaimId>)> =
            if let Some(existing) = self.evidences.get(&input.evidence_id) {
                if is_deterministic && !self.is_valid_deterministic_evidence(existing) {
                    Some((existing.artifact_id, existing.claim_id))
                } else {
                    if existing.artifact_id == input.artifact_id
                        && existing.claim_id == input.claim_id
                        && existing.kind == input.kind
                        && existing.excerpt == input.excerpt
                        && existing.observed_at == input.observed_at
                    {
                        return Ok(None);
                    }
                    return Err(DomainError::DuplicateId {
                        kind: "evidence",
                        id: input.evidence_id.value(),
                    });
                }
            } else {
                None
            };

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

        // ── Safe to mutate: remove malformed existing if needed ──
        if let Some((old_artifact_id, old_claim_id)) = should_replace {
            self.evidences.remove(&input.evidence_id);
            if let Some(artifact) = self.artifacts.get_mut(&old_artifact_id) {
                artifact.evidence_ids.remove(&input.evidence_id);
            }
            if let Some(claim_id) = old_claim_id
                && let Some(claim) = self.claims.get_mut(&claim_id)
            {
                claim.evidence_ids.remove(&input.evidence_id);
            }
        }

        let kind = input.kind.clone();
        self.evidences.insert(
            input.evidence_id,
            Evidence::new(
                input.evidence_id,
                input.artifact_id,
                input.claim_id,
                kind.clone(),
                input.excerpt.clone(),
                input.observed_at,
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
        })))
    }

    // ── Replay apply ─────────────────────────────────────────────

    pub(crate) fn apply_evidence_recorded(
        &mut self,
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: &EvidenceKind,
        excerpt: &str,
        observed_at: LogicalTick,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        // Determine what to do with an existing record at this ID.
        // Read-only decision — mutation happens only after all
        // validations pass.
        let should_replace: Option<(ArtifactId, Option<ClaimId>)> =
            if let Some(existing) = self.evidences.get(&evidence_id) {
                let is_deterministic = self
                    .chunks
                    .values()
                    .any(|chunk| evidence_id_for(chunk.artifact_id, chunk.order) == evidence_id);
                let existing_is_malformed = is_deterministic
                    && !self.is_valid_deterministic_record(
                        existing.id,
                        existing.artifact_id,
                        &existing.kind,
                    );
                let incoming_is_valid = is_deterministic
                    && self.is_valid_deterministic_record(evidence_id, artifact_id, kind);
                if existing_is_malformed && incoming_is_valid {
                    Some((existing.artifact_id, existing.claim_id))
                } else if existing.artifact_id == artifact_id
                    && existing.claim_id == claim_id
                    && existing.kind == *kind
                    && existing.excerpt == excerpt
                    && existing.observed_at == observed_at
                {
                    // Identical valid record — leave state
                    // unchanged, fall through to event_log.push
                    // so the replay sequence is preserved.
                    None
                } else {
                    return Err(DomainError::DuplicateId {
                        kind: "evidence",
                        id: evidence_id.value(),
                    });
                }
            } else {
                None
            };

        // Validate incoming claim BEFORE any mutation, so a
        // failed replacement does not leave state corrupted.
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

        // ── Safe to mutate: remove malformed existing if needed ──
        if let Some((old_artifact_id, old_claim_id)) = should_replace {
            self.evidences.remove(&evidence_id);
            if let Some(artifact) = self.artifacts.get_mut(&old_artifact_id) {
                artifact.evidence_ids.remove(&evidence_id);
            }
            if let Some(cid) = old_claim_id
                && let Some(claim) = self.claims.get_mut(&cid)
            {
                claim.evidence_ids.remove(&evidence_id);
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
            ),
        );
        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.evidence_ids.insert(evidence_id);
        }
        if let Some(claim_id) = claim_id
            && let Some(claim) = self.claims.get_mut(&claim_id)
        {
            claim.evidence_ids.insert(evidence_id);
        }
        Ok(())
    }
}
