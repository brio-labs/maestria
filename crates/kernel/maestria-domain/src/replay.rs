use crate::types::*;
use std::collections::BTreeSet;
impl KernelState {
    pub fn apply_event(&mut self, envelope: DomainEventEnvelope) -> Result<(), DomainError> {
        let expected_id = self.event_log.len() as u64 + 1;
        if envelope.id.value() != expected_id {
            return Err(DomainError::InvalidEventId {
                expected: expected_id,
                actual: envelope.id.value(),
            });
        }
        if envelope.sequence.value() != expected_id {
            return Err(DomainError::InvalidSequence {
                expected: expected_id,
                actual: envelope.sequence.value(),
            });
        }
        match &envelope.event {
            DomainEvent::ArtifactRegistered { artifact_id, title } => {
                if self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "artifact",
                        id: artifact_id.value(),
                    });
                }
                self.artifacts.insert(
                    *artifact_id,
                    Artifact::with_title(*artifact_id, title.clone()),
                );
            }
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
                text,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.chunks.contains_key(chunk_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "chunk",
                        id: chunk_id.value(),
                    });
                }
                if self
                    .chunks
                    .values()
                    .any(|chunk| chunk.artifact_id == *artifact_id && chunk.order == *order)
                {
                    return Err(DomainError::DuplicateId {
                        kind: "chunk_order",
                        id: chunk_id.value(),
                    });
                }
                self.chunks.insert(
                    *chunk_id,
                    Chunk::new(*chunk_id, *artifact_id, *order, text.clone()),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.chunk_ids.insert(*chunk_id);
                }
                if let Some(artifact) = self.artifacts.get(artifact_id)
                    && artifact.index_status == IndexStatus::Pending
                {
                    self.pending_full_text.insert(*chunk_id);
                }
            }
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                title,
                body,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.cards.contains_key(card_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "card",
                        id: card_id.value(),
                    });
                }
                self.cards.insert(
                    *card_id,
                    Card::new(*card_id, *artifact_id, title.clone(), body.clone()),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.card_ids.insert(*card_id);
                }
            }
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                if self.claims.contains_key(claim_id) {
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
                    if evidence.artifact_id != *artifact_id {
                        return Err(DomainError::ArtifactMismatch {
                            expected: *artifact_id,
                            actual: evidence.artifact_id,
                        });
                    }
                    if let Some(existing_claim) = evidence.claim_id
                        && existing_claim != *claim_id
                    {
                        return Err(DomainError::DuplicateId {
                            kind: "evidence_claim",
                            id: evidence_id.value(),
                        });
                    }
                }
                let mut claim = Claim::new(*claim_id, *artifact_id, text.clone());
                claim.evidence_ids.extend(evidence_ids.iter().copied());
                for evidence_id in evidence_ids {
                    if let Some(evidence) = self.evidences.get_mut(evidence_id) {
                        evidence.claim_id = Some(*claim_id);
                    }
                }
                self.claims.insert(*claim_id, claim);
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.claim_ids.insert(*claim_id);
                }
            }
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
                excerpt,
                observed_at,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                // Determine what to do with an existing record at this ID.
                // Read-only decision — mutation happens only after all
                // validations pass.
                let should_replace: Option<(ArtifactId, Option<ClaimId>)> =
                    if let Some(existing) = self.evidences.get(evidence_id) {
                        let is_deterministic = self.chunks.values().any(|chunk| {
                            crate::evidence_id_for(chunk.artifact_id, chunk.order) == *evidence_id
                        });
                        let existing_is_malformed = is_deterministic
                            && !self.is_valid_deterministic_record(
                                existing.id,
                                existing.artifact_id,
                                &existing.kind,
                            );
                        let incoming_is_valid = is_deterministic
                            && self.is_valid_deterministic_record(*evidence_id, *artifact_id, kind);
                        if existing_is_malformed && incoming_is_valid {
                            Some((existing.artifact_id, existing.claim_id))
                        } else if existing.artifact_id == *artifact_id
                            && existing.claim_id == *claim_id
                            && existing.kind == *kind
                            && existing.excerpt == *excerpt
                            && existing.observed_at == *observed_at
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
                        .get(claim_id)
                        .ok_or(DomainError::MissingClaim { id: *claim_id })?;
                    if claim.artifact_id != *artifact_id {
                        return Err(DomainError::ArtifactMismatch {
                            expected: *artifact_id,
                            actual: claim.artifact_id,
                        });
                    }
                }

                // ── Safe to mutate: remove malformed existing if needed ──
                if let Some((old_artifact_id, old_claim_id)) = should_replace {
                    self.evidences.remove(evidence_id);
                    if let Some(artifact) = self.artifacts.get_mut(&old_artifact_id) {
                        artifact.evidence_ids.remove(evidence_id);
                    }
                    if let Some(cid) = old_claim_id
                        && let Some(claim) = self.claims.get_mut(&cid)
                    {
                        claim.evidence_ids.remove(evidence_id);
                    }
                }

                // Insert new evidence and reverse links.
                self.evidences.insert(
                    *evidence_id,
                    Evidence::new(
                        *evidence_id,
                        *artifact_id,
                        *claim_id,
                        kind.clone(),
                        excerpt.clone(),
                        *observed_at,
                    ),
                );
                if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                    artifact.evidence_ids.insert(*evidence_id);
                }
                if let Some(claim_id) = claim_id
                    && let Some(claim) = self.claims.get_mut(claim_id)
                {
                    claim.evidence_ids.insert(*evidence_id);
                }
            }
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => {
                if self.tasks.contains_key(task_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "task",
                        id: task_id.value(),
                    });
                }
                if let Some(art_id) = artifact_id
                    && !self.artifacts.contains_key(art_id)
                {
                    return Err(DomainError::MissingArtifact { id: *art_id });
                }
                let mut task = Task::new(*task_id, title.clone(), *priority);
                if let Some(art_id) = artifact_id {
                    task.artifact_ids.insert(*art_id);
                }
                self.tasks.insert(*task_id, task);
            }
            DomainEvent::TaskStatusChanged { task_id, from, to } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(DomainError::MissingTask { id: *task_id })?;
                if task.status != *from {
                    return Err(DomainError::InvalidTaskTransition {
                        task_id: *task_id,
                        from: task.status,
                        to: *from,
                    });
                }
                if *from == *to && !from.is_completion() {
                    return Err(DomainError::InvalidTaskTransition {
                        task_id: *task_id,
                        from: *from,
                        to: *to,
                    });
                }
                if *from != *to {
                    if to.is_completion() {
                        return Err(DomainError::ValidationRequired { task_id: *task_id });
                    }
                    if !from.can_transition_to(*to) {
                        return Err(DomainError::InvalidTaskTransition {
                            task_id: *task_id,
                            from: *from,
                            to: *to,
                        });
                    }
                    task.status = *to;
                }
            }
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(DomainError::MissingTask { id: *task_id })?;
                let report = self.validation_reports.get(validation_report_id).ok_or(
                    DomainError::MissingValidationReport {
                        id: *validation_report_id,
                    },
                )?;
                if report.task_id != Some(*task_id) {
                    return Err(DomainError::ValidationReportTaskMismatch {
                        report_id: *validation_report_id,
                        report_task_id: report.task_id,
                        task_id: *task_id,
                    });
                }
                if !report.passed {
                    return Err(DomainError::ValidationFailed { task_id: *task_id });
                }
                if *status == TaskStatus::CompletedVerified && !report.warnings.is_empty() {
                    return Err(DomainError::ValidationWarningsForbidden { task_id: *task_id });
                }
                if *status == TaskStatus::CompletedWithWarnings && report.warnings.is_empty() {
                    return Err(DomainError::ValidationWarningsRequired { task_id: *task_id });
                }
                if *status != TaskStatus::CompletedVerified
                    && *status != TaskStatus::CompletedWithWarnings
                {
                    return Err(DomainError::InvalidTaskTransition {
                        task_id: *task_id,
                        from: task.status,
                        to: *status,
                    });
                }
                if !task.status.can_transition_to(*status) {
                    return Err(DomainError::InvalidTaskTransition {
                        task_id: *task_id,
                        from: task.status,
                        to: *status,
                    });
                }
                task.status = *status;
                task.validation_report_id = Some(*validation_report_id);
            }
            DomainEvent::ClaimValidationUpdated { claim_id, status } => {
                let claim = self
                    .claims
                    .get_mut(claim_id)
                    .ok_or(DomainError::MissingClaim { id: *claim_id })?;
                claim.status = status.clone();
            }
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => {
                let claim = self
                    .claims
                    .get_mut(claim_id)
                    .ok_or(DomainError::MissingClaim { id: *claim_id })?;
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
                if let Some(existing_claim) = evidence.claim_id
                    && existing_claim != *claim_id
                {
                    return Err(DomainError::DuplicateId {
                        kind: "evidence_claim",
                        id: evidence_id.value(),
                    });
                }
                claim.evidence_ids.insert(*evidence_id);
                if let Some(evidence) = self.evidences.get_mut(evidence_id) {
                    evidence.claim_id = Some(*claim_id);
                }
            }
            DomainEvent::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => {
                if *confidence_milli > 1000 {
                    return Err(DomainError::InvalidConfidence {
                        max: 1000,
                        actual: *confidence_milli,
                    });
                }
                let validate_endpoint = |endpoint: &RelationEndpoint| -> Result<(), DomainError> {
                    match endpoint {
                        RelationEndpoint::Artifact(id) => {
                            if !self.artifacts.contains_key(id) {
                                return Err(DomainError::MissingArtifact { id: *id });
                            }
                        }
                        RelationEndpoint::Claim(id) => {
                            if !self.claims.contains_key(id) {
                                return Err(DomainError::MissingClaim { id: *id });
                            }
                        }
                        RelationEndpoint::Task(id) => {
                            if !self.tasks.contains_key(id) {
                                return Err(DomainError::MissingTask { id: *id });
                            }
                        }
                        RelationEndpoint::Memory(id) => {
                            if !self.memories.contains_key(id) {
                                return Err(DomainError::MissingMemory { id: *id });
                            }
                        }
                        RelationEndpoint::Card(id) => {
                            if !self.cards.contains_key(id) {
                                return Err(DomainError::MissingCard { id: *id });
                            }
                        }
                    }
                    Ok(())
                };
                validate_endpoint(source)?;
                validate_endpoint(target)?;
                if self.relations.contains_key(relation_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "relation",
                        id: relation_id.value(),
                    });
                }
                if let Some(ev_id) = evidence_id
                    && !self.evidences.contains_key(ev_id)
                {
                    return Err(DomainError::MissingEvidence { id: *ev_id });
                }
                self.relations.insert(
                    *relation_id,
                    Relation {
                        id: *relation_id,
                        source: *source,
                        kind: *kind,
                        target: *target,
                        evidence_id: *evidence_id,
                        confidence_milli: *confidence_milli,
                    },
                );
            }
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => {
                if *confidence_milli > 1000 {
                    return Err(DomainError::InvalidConfidence {
                        max: 1000,
                        actual: *confidence_milli,
                    });
                }
                if self.memory_candidates.contains_key(candidate_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "memory_candidate",
                        id: candidate_id.value(),
                    });
                }
                let claim = self
                    .claims
                    .get(claim_id)
                    .ok_or(DomainError::MissingClaim { id: *claim_id })?;
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
                    *candidate_id,
                    MemoryCandidate {
                        id: *candidate_id,
                        claim_id: *claim_id,
                        evidence_ids: evidence_ids.clone(),
                        confidence_milli: *confidence_milli,
                    },
                );
            }
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
            } => {
                if self.memories.contains_key(memory_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "memory",
                        id: memory_id.value(),
                    });
                }
                let candidate = self
                    .memory_candidates
                    .get(candidate_id)
                    .ok_or(DomainError::MissingMemoryCandidate { id: *candidate_id })?;
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
                    *memory_id,
                    Memory {
                        id: *memory_id,
                        candidate_id: *candidate_id,
                        claim_id: candidate.claim_id,
                        evidence_ids: candidate.evidence_ids.clone(),
                        status: MemoryStatus::Active,
                    },
                );
            }
            DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => {
                if !self
                    .memory_candidates
                    .contains_key(contradicting_candidate_id)
                {
                    return Err(DomainError::MissingMemoryCandidate {
                        id: *contradicting_candidate_id,
                    });
                }
                let memory = self
                    .memories
                    .get_mut(memory_id)
                    .ok_or(DomainError::MissingMemory { id: *memory_id })?;
                memory.status = MemoryStatus::Contradicted;
            }
            DomainEvent::MemoryDeprecated { memory_id } => {
                let memory = self
                    .memories
                    .get_mut(memory_id)
                    .ok_or(DomainError::MissingMemory { id: *memory_id })?;
                memory.status = MemoryStatus::Deprecated;
            }
            DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => {
                if !self.memories.contains_key(by_memory_id) {
                    return Err(DomainError::MissingMemory { id: *by_memory_id });
                }
                let memory = self
                    .memories
                    .get_mut(memory_id)
                    .ok_or(DomainError::MissingMemory { id: *memory_id })?;
                memory.status = MemoryStatus::Superseded;
            }
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => {
                if self.validation_reports.contains_key(report_id) {
                    return Err(DomainError::DuplicateId {
                        kind: "validation_report",
                        id: report_id.value(),
                    });
                }
                if let Some(tid) = task_id
                    && !self.tasks.contains_key(tid)
                {
                    return Err(DomainError::MissingTask { id: *tid });
                }
                self.validation_reports.insert(
                    *report_id,
                    ValidationReportRecord {
                        task_id: *task_id,
                        passed: *passed,
                        warnings: warnings.clone(),
                    },
                );
            }
            DomainEvent::UserIntentObserved { task_id, title } => {
                if title.trim().is_empty() {
                    return Err(DomainError::EmptyIntent);
                }
                if !self.tasks.contains_key(task_id) {
                    return Err(DomainError::MissingTask { id: *task_id });
                }
            }
            DomainEvent::ParserStarted {
                artifact_id,
                title,
                source_path,
                content_hash,
                blob_id,
            } => {
                // Reconstruct pending-parser metadata so the daemon can find
                // stranded artifacts on restart and re-drive parsing.
                self.pending_parsers.insert(
                    *artifact_id,
                    ParserStarted {
                        artifact_id: *artifact_id,
                        title: title.clone(),
                        source_path: source_path.clone(),
                        content_hash: content_hash.clone(),
                        blob_id: *blob_id,
                    },
                );
            }
            DomainEvent::ArtifactParsed { artifact_id, .. } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                // pending_parsers is NOT removed here — it stays until
                // terminal ArtifactIndexed so a crash before evidence
                // leaves the parser retryable.
                // Mark as parsed so resume duplicate suppression works.
                self.parsed_artifact_ids.insert(*artifact_id);
            }
            DomainEvent::SearchCompleted { artifact_id, .. } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                // SearchCompleted must never touch pending parser metadata.
            }
            DomainEvent::PendingIndex {
                artifact_id,
                content_hash,
            } => {
                let artifact = self
                    .artifacts
                    .get_mut(artifact_id)
                    .ok_or(DomainError::MissingArtifact { id: *artifact_id })?;
                artifact.content_hash = Some(content_hash.clone());
                artifact.index_status = IndexStatus::Pending;
            }
            DomainEvent::FullTextIndexed {
                artifact_id,
                chunk_id,
            } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                let chunk = self
                    .chunks
                    .get(chunk_id)
                    .ok_or(DomainError::MissingChunk { id: *chunk_id })?;
                if chunk.artifact_id != *artifact_id {
                    return Err(DomainError::ArtifactMismatch {
                        expected: *artifact_id,
                        actual: chunk.artifact_id,
                    });
                }
                self.pending_full_text.remove(chunk_id);
            }
            DomainEvent::ArtifactIndexed { artifact_id } => {
                if !self.artifacts.contains_key(artifact_id) {
                    return Err(DomainError::MissingArtifact { id: *artifact_id });
                }
                let has_pending = self.chunks.values().any(|c| {
                    c.artifact_id == *artifact_id && self.pending_full_text.contains(&c.id)
                });
                if has_pending {
                    return Err(DomainError::PendingChunksExist {
                        artifact_id: *artifact_id,
                    });
                }
                // Only terminalize when evidence is source-backed and complete.
                // Invalid ArtifactIndexed events (e.g. from a corrupt log) are
                // still appended to the event log but leave the artifact Pending
                // and retain recovery metadata so retry/resume can regenerate evidence.
                if self.evidence_complete_for(*artifact_id) {
                    let artifact = self
                        .artifacts
                        .get_mut(artifact_id)
                        .ok_or(DomainError::MissingArtifact { id: *artifact_id })?;
                    artifact.index_status = IndexStatus::Indexed;
                    // Terminal indexing frees the pending parser entry.
                    self.pending_parsers.remove(artifact_id);
                } else {
                    // Remove each invalid deterministic source-evidence record
                    // so ResumeParser can regenerate and RecordEvidence can
                    // replace it without tripping duplicate-ID rejection.
                    let expected_hash = self
                        .artifacts
                        .get(artifact_id)
                        .and_then(|a| a.content_hash.as_deref());
                    // Collect (evidence_id, actual_owner_artifact_id,
                    // claim_id) so we can repair cross-artifact references
                    // and claim reverse-links without borrow conflicts.
                    let mut to_remove: Vec<(EvidenceId, ArtifactId, Option<ClaimId>)> = Vec::new();
                    for chunk in self.chunks.values() {
                        if chunk.artifact_id != *artifact_id {
                            continue;
                        }
                        let expected_id = crate::evidence_id_for(chunk.artifact_id, chunk.order);
                        if let Some(ev) = self.evidences.get(&expected_id) {
                            let is_valid = ev.artifact_id == *artifact_id
                                && matches!(
                                    &ev.kind,
                                    EvidenceKind::FileSpan {
                                        content_hash,
                                        snapshot: Some(_),
                                        ..
                                    } if expected_hash == Some(content_hash.as_str())
                                );
                            if !is_valid {
                                to_remove.push((expected_id, ev.artifact_id, ev.claim_id));
                            }
                        }
                    }
                    if !to_remove.is_empty() {
                        // Remove all invalid records from the evidence map
                        // first — no borrow conflict with artifacts or claims.
                        for (id, _owner, _claim_id) in &to_remove {
                            self.evidences.remove(id);
                        }
                        // Clean up the ArtifactIndexed target artifact.
                        if let Some(artifact) = self.artifacts.get_mut(artifact_id) {
                            for (id, _owner, _claim_id) in &to_remove {
                                artifact.evidence_ids.remove(id);
                            }
                        }
                        // Clean up each actual owner artifact when it
                        // differs from the indexed target (cross-artifact
                        // malformed evidence).
                        for (id, owner_id, _claim_id) in &to_remove {
                            if *owner_id != *artifact_id
                                && let Some(owner) = self.artifacts.get_mut(owner_id)
                            {
                                owner.evidence_ids.remove(id);
                            }
                        }
                        // Clean up claim reverse-links so no dangling
                        // evidence-ID references remain.
                        for (id, _owner_id, claim_id) in &to_remove {
                            if let Some(cid) = claim_id
                                && let Some(claim) = self.claims.get_mut(cid)
                            {
                                claim.evidence_ids.remove(id);
                            }
                        }
                    }
                }
            }
            DomainEvent::HarnessRunCompleted { task_id, .. } => {
                if let Some(task_id) = task_id
                    && !self.tasks.contains_key(task_id)
                {
                    return Err(DomainError::MissingTask { id: *task_id });
                }
            }
            DomainEvent::ApprovalRecorded { task_id, .. } => {
                if !self.tasks.contains_key(task_id) {
                    return Err(DomainError::MissingTask { id: *task_id });
                }
            }
            DomainEvent::TickObserved { .. } => {}
        }

        self.event_log.push(envelope);
        Ok(())
    }
    /// Returns `true` when the record represents a valid deterministic
    /// source-evidence record: its ID maps to a known chunk, its
    /// `artifact_id` matches the chunk owner, its kind is `FileSpan`
    /// with a `Some` snapshot, and its `content_hash` equals the
    /// artifact's recorded content hash.
    fn is_valid_deterministic_record(
        &self,
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        kind: &EvidenceKind,
    ) -> bool {
        let Some(chunk) = self
            .chunks
            .values()
            .find(|chunk| crate::evidence_id_for(chunk.artifact_id, chunk.order) == evidence_id)
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
}

/// Replay a deterministic input sequence into a fresh state.
pub fn replay_inputs(
    inputs: &[DomainInput],
) -> Result<(KernelState, Vec<DomainEventEnvelope>, Vec<MaestriaEffect>), DomainError> {
    let mut state = KernelState::new();
    let mut events = Vec::new();
    let mut effects = Vec::new();

    for input in inputs {
        let output = state.apply_input(input.clone())?;
        events.extend(output.events);
        effects.extend(output.effects);
    }

    Ok((state, events, effects))
}

/// Replay a deterministic event log into a fresh state.
pub fn replay_events(envelopes: &[DomainEventEnvelope]) -> Result<KernelState, DomainError> {
    let mut state = KernelState::new();
    for envelope in envelopes {
        state.apply_event(envelope.clone())?;
    }
    Ok(state)
}
