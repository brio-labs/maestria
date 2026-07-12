use std::collections::BTreeSet;

use crate::types::*;

impl KernelState {
    pub(super) fn handle_register_artifact(
        &mut self,
        input: RegisterArtifactInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::DuplicateId {
                kind: "artifact",
                id: input.artifact_id.value(),
            });
        }
        self.artifacts.insert(
            input.artifact_id,
            Artifact::with_title(input.artifact_id, input.title.clone()),
        );
        Ok(self.emit_event(DomainEvent::ArtifactRegistered {
            artifact_id: input.artifact_id,
            title: input.title,
        }))
    }

    pub(super) fn handle_register_chunk(
        &mut self,
        input: RegisterChunkInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if self.chunks.contains_key(&input.chunk_id) {
            return Err(DomainError::DuplicateId {
                kind: "chunk",
                id: input.chunk_id.value(),
            });
        }
        if self
            .chunks
            .values()
            .any(|chunk| chunk.artifact_id == input.artifact_id && chunk.order == input.order)
        {
            return Err(DomainError::DuplicateId {
                kind: "chunk_order",
                id: input.chunk_id.value(),
            });
        }

        let chunk = Chunk::new(
            input.chunk_id,
            input.artifact_id,
            input.order,
            input.text.clone(),
        );
        self.chunks.insert(input.chunk_id, chunk);
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.chunk_ids.insert(input.chunk_id);
        }

        Ok(self.emit_event(DomainEvent::ChunkRegistered {
            chunk_id: input.chunk_id,
            artifact_id: input.artifact_id,
            order: input.order,
            text: input.text,
        }))
    }

    pub(super) fn handle_create_card(
        &mut self,
        input: CreateCardInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.cards.contains_key(&input.card_id) {
            return Err(DomainError::DuplicateId {
                kind: "card",
                id: input.card_id.value(),
            });
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        self.cards.insert(
            input.card_id,
            Card::new(
                input.card_id,
                input.artifact_id,
                input.title.clone(),
                input.body.clone(),
            ),
        );

        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.card_ids.insert(input.card_id);
        }

        Ok(self.emit_event(DomainEvent::CardCreated {
            card_id: input.card_id,
            artifact_id: input.artifact_id,
            title: input.title,
            body: input.body,
        }))
    }

    pub(super) fn handle_record_evidence(
        &mut self,
        input: RecordEvidenceInput,
    ) -> Result<Option<DomainEventEnvelope>, DomainError> {
        if let Some(existing) = self.evidences.get(&input.evidence_id) {
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

    pub(super) fn handle_open_task(
        &mut self,
        input: OpenTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.tasks.contains_key(&input.task_id) {
            return Err(DomainError::DuplicateId {
                kind: "task",
                id: input.task_id.value(),
            });
        }
        if let Some(artifact_id) = input.artifact_id
            && !self.artifacts.contains_key(&artifact_id)
        {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }

        let task = Task::new(input.task_id, input.title.clone(), input.priority);
        let artifact_id = input.artifact_id;
        self.tasks.insert(input.task_id, task);
        if let Some(artifact_id) = artifact_id
            && let Some(task) = self.tasks.get_mut(&input.task_id)
        {
            task.artifact_ids.insert(artifact_id);
        }

        Ok(self.emit_event(DomainEvent::TaskOpened {
            task_id: input.task_id,
            title: input.title,
            priority: input.priority,
            artifact_id: input.artifact_id,
        }))
    }

    pub(super) fn handle_change_task_status(
        &mut self,
        task_id: TaskId,
        to: TaskStatus,
    ) -> Result<(TaskStatus, TaskStatus), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        let from = task.status;
        if to.is_completion() {
            return Err(DomainError::ValidationRequired { task_id });
        }
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition { task_id, from, to });
        }
        task.status = to;
        Ok((from, to))
    }

    pub(super) fn handle_complete_task(
        &mut self,
        input: CompleteTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let task = self
            .tasks
            .get_mut(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;
        let report = self
            .validation_reports
            .get(&input.validation_report_id)
            .ok_or(DomainError::MissingValidationReport {
                id: input.validation_report_id,
            })?;
        if report.task_id != Some(input.task_id) {
            return Err(DomainError::ValidationReportTaskMismatch {
                report_id: input.validation_report_id,
                report_task_id: report.task_id,
                task_id: input.task_id,
            });
        }
        let from = task.status;
        if !report.passed {
            return Err(DomainError::ValidationFailed {
                task_id: input.task_id,
            });
        }
        let to = if report.warnings.is_empty() {
            TaskStatus::CompletedVerified
        } else {
            TaskStatus::CompletedWithWarnings
        };
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition {
                task_id: input.task_id,
                from,
                to,
            });
        }
        task.status = to;
        task.validation_report_id = Some(input.validation_report_id);
        Ok(self.emit_event(DomainEvent::TaskCompletionRecorded {
            task_id: input.task_id,
            status: to,
            validation_report_id: input.validation_report_id,
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

    pub(super) fn handle_create_relation(
        &mut self,
        input: CreateRelationInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if input.confidence_milli > 1000 {
            return Err(DomainError::InvalidConfidence {
                max: 1000,
                actual: input.confidence_milli,
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
        validate_endpoint(&input.source)?;
        validate_endpoint(&input.target)?;
        if self.relations.contains_key(&input.relation_id) {
            return Err(DomainError::DuplicateId {
                kind: "relation",
                id: input.relation_id.value(),
            });
        }
        if let Some(evidence_id) = input.evidence_id
            && !self.evidences.contains_key(&evidence_id)
        {
            return Err(DomainError::MissingEvidence { id: evidence_id });
        }
        let relation = Relation {
            id: input.relation_id,
            source: input.source,
            kind: input.kind,
            target: input.target,
            evidence_id: input.evidence_id,
            confidence_milli: input.confidence_milli,
        };
        self.relations.insert(input.relation_id, relation);
        Ok(self.emit_event(DomainEvent::RelationCreated {
            relation_id: input.relation_id,
            source: input.source,
            kind: input.kind,
            target: input.target,
            evidence_id: input.evidence_id,
            confidence_milli: input.confidence_milli,
        }))
    }

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

    pub(super) fn handle_record_validation_report(
        &mut self,
        input: RecordValidationReportInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.validation_reports.contains_key(&input.report_id) {
            return Err(DomainError::DuplicateId {
                kind: "validation_report",
                id: input.report_id.value(),
            });
        }
        if let Some(task_id) = input.task_id
            && !self.tasks.contains_key(&task_id)
        {
            return Err(DomainError::MissingTask { id: task_id });
        }
        self.validation_reports.insert(
            input.report_id,
            ValidationReportRecord {
                task_id: input.task_id,
                passed: input.passed,
                warnings: input.warnings.clone(),
            },
        );
        Ok(self.emit_event(DomainEvent::ValidationReportCreated {
            report_id: input.report_id,
            task_id: input.task_id,
            passed: input.passed,
            warnings: input.warnings,
        }))
    }

    pub(super) fn handle_user_intent(
        &mut self,
        input: UserIntent,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if input.title.trim().is_empty() {
            return Err(DomainError::EmptyIntent);
        }

        let open = self.handle_open_task(OpenTaskInput {
            task_id: input.task_id,
            title: input.title.clone(),
            priority: input.priority,
            artifact_id: None,
        })?;

        let observed = self.emit_event(DomainEvent::UserIntentObserved {
            task_id: input.task_id,
            title: input.title,
        });

        Ok(vec![open, observed])
    }

    pub(super) fn handle_parser_completed(
        &mut self,
        input: ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();

        // First-time commit from fresh detection (pending_artifacts).
        // On fresh ingestion this block fires once; on retry or resume
        // the pending_artifacts entry is absent so this is skipped.
        if let Some(pending) = self.pending_artifacts.remove(&input.artifact_id) {
            use std::collections::btree_map::Entry;
            if let Entry::Vacant(entry) = self.artifacts.entry(input.artifact_id) {
                entry.insert(Artifact::with_title(
                    input.artifact_id,
                    pending.title.clone(),
                ));
                let register_event = self.emit_event(DomainEvent::ArtifactRegistered {
                    artifact_id: input.artifact_id,
                    title: pending.title,
                });
                generated.push(register_event);
            }
            // Set content_hash and status on the artifact regardless of whether
            // it was just created or already existed (e.g. from replay).
            if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                artifact.content_hash = Some(pending.content_hash.clone());
                artifact.index_status = IndexStatus::Pending;
            }
            let pending_event = self.emit_event(DomainEvent::PendingIndex {
                artifact_id: input.artifact_id,
                content_hash: pending.content_hash,
            });
            generated.push(pending_event);
        }

        // Resume path: no pending_artifacts (in-memory only, lost on restart),
        // but pending_parsers survived via replay. Create the artifact from the
        // durable parser metadata so chunk/card registration has a home.
        if !self.artifacts.contains_key(&input.artifact_id)
            && let Some(parser) = self.pending_parsers.get(&input.artifact_id).cloned()
        {
            self.artifacts.insert(
                input.artifact_id,
                Artifact::with_title(input.artifact_id, parser.title.clone()),
            );
            let register_event = self.emit_event(DomainEvent::ArtifactRegistered {
                artifact_id: input.artifact_id,
                title: parser.title.clone(),
            });
            generated.push(register_event);
            if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                artifact.content_hash = Some(parser.content_hash.clone());
                artifact.index_status = IndexStatus::Pending;
            }
            let pending_event = self.emit_event(DomainEvent::PendingIndex {
                artifact_id: input.artifact_id,
                content_hash: parser.content_hash,
            });
            generated.push(pending_event);
        }

        // Remove durable pending-parser metadata now that parsing succeeded.
        // On fresh ingestion this drops the ParserStarted entry; on resume it
        // drops the entry reconstructed from replay. Either way the artifact
        // is no longer stranded.
        self.pending_parsers.remove(&input.artifact_id);

        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut new_chunks = 0u32;
        for chunk in &input.chunks {
            if let Some(existing) = self.chunks.get(&chunk.chunk_id) {
                if existing.artifact_id != chunk.artifact_id
                    || existing.order != chunk.order
                    || existing.text != chunk.text
                {
                    return Err(DomainError::DuplicateId {
                        kind: "chunk",
                        id: chunk.chunk_id.value(),
                    });
                }
            } else {
                let envelope = self.handle_register_chunk(chunk.clone())?;
                generated.push(envelope);
                self.pending_full_text.insert(chunk.chunk_id);
                new_chunks += 1;
            }
        }

        for card in input.cards {
            if let Some(existing) = self.cards.get(&card.card_id) {
                if existing.artifact_id != card.artifact_id
                    || existing.title != card.title
                    || existing.body != card.body
                {
                    return Err(DomainError::DuplicateId {
                        kind: "card",
                        id: card.card_id.value(),
                    });
                }
            } else {
                generated.push(self.handle_create_card(card)?);
            }
        }

        let parsed = self.emit_event(DomainEvent::ArtifactParsed {
            artifact_id: input.artifact_id,
            chunks_added: new_chunks,
        });
        generated.push(parsed);

        Ok(generated)
    }

    pub(super) fn handle_search_completed(
        &mut self,
        input: SearchResultSet,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut generated = Vec::new();
        for card in input.cards {
            generated.push(self.handle_create_card(card)?);
        }

        let cards_added = (generated.len().min(u32::MAX as usize)) as u32;
        let event = self.emit_event(DomainEvent::SearchCompleted {
            artifact_id: input.artifact_id,
            cards_added,
        });
        generated.push(event);
        Ok(generated)
    }

    pub(super) fn handle_harness_completed(
        &mut self,
        input: HarnessRunCompleted,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();
        let task_id = input.task_id;
        let exit_code = input.exit_code;
        if let Some(task_id) = task_id
            && !self.tasks.contains_key(&task_id)
        {
            return Err(DomainError::MissingTask { id: task_id });
        }

        let base_event = self.emit_event(DomainEvent::HarnessRunCompleted {
            task_id,
            command: input.command,
            exit_code,
        });
        generated.push(base_event);

        if let Some(task_id) = task_id
            && let Some(task) = self.tasks.get(&task_id)
        {
            if input.exit_code != 0 && task.status.can_transition_to(TaskStatus::Blocked) {
                let (from, to) = self.handle_change_task_status(task_id, TaskStatus::Blocked)?;
                generated.push(self.emit_event(DomainEvent::TaskStatusChanged {
                    task_id,
                    from,
                    to,
                }));
            } else if input.exit_code == 0 && task.status == TaskStatus::Draft {
                let (from, to) = self.handle_change_task_status(task_id, TaskStatus::Open)?;
                generated.push(self.emit_event(DomainEvent::TaskStatusChanged {
                    task_id,
                    from,
                    to,
                }));
            }
        }

        if input.exit_code != 0
            && let Some(task_id) = task_id
        {
            generated.push(self.emit_event(DomainEvent::ApprovalRecorded {
                task_id,
                approved: false,
            }));
        }

        Ok(generated)
    }

    pub(super) fn handle_validation_completed(
        &mut self,
        input: ValidationCompleted,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let status = if input.valid {
            ClaimStatus::Verified
        } else {
            ClaimStatus::Disputed
        };

        let claim = self
            .claims
            .get_mut(&input.claim_id)
            .ok_or(DomainError::MissingClaim { id: input.claim_id })?;
        claim.status = status.clone();

        Ok(self.emit_event(DomainEvent::ClaimValidationUpdated {
            claim_id: input.claim_id,
            status,
        }))
    }

    pub(super) fn handle_approval_resolved(
        &mut self,
        input: ApprovalDecision,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let task = self
            .tasks
            .get(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;

        let target = if input.approved {
            match task.status {
                TaskStatus::Draft | TaskStatus::Open | TaskStatus::Blocked => TaskStatus::Active,
                _ => task.status,
            }
        } else {
            TaskStatus::Blocked
        };
        let (from, to) = if task.status == target {
            (task.status, task.status)
        } else {
            self.handle_change_task_status(input.task_id, target)?
        };
        let mut emitted = vec![];
        if from != to {
            emitted.push(self.emit_event(DomainEvent::TaskStatusChanged {
                task_id: input.task_id,
                from,
                to,
            }));
        }
        emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
            task_id: input.task_id,
            approved: input.approved,
        }));
        Ok(emitted)
    }

    pub(super) fn handle_start_full_text_index(
        &mut self,
        input: &StartFullTextIndex,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        Ok(())
    }
    pub(super) fn handle_full_text_index_completed(
        &mut self,
        input: FullTextIndexCompleted,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if !self.chunks.contains_key(&input.chunk_id) {
            return Err(DomainError::MissingChunk { id: input.chunk_id });
        }
        let chunk = self
            .chunks
            .get(&input.chunk_id)
            .ok_or(DomainError::MissingChunk { id: input.chunk_id })?;
        if chunk.artifact_id != input.artifact_id {
            return Err(DomainError::ArtifactMismatch {
                expected: input.artifact_id,
                actual: chunk.artifact_id,
            });
        }

        let mut generated = Vec::new();

        if self.pending_full_text.remove(&input.chunk_id) {
            generated.push(self.emit_event(DomainEvent::FullTextIndexed {
                artifact_id: input.artifact_id,
                chunk_id: input.chunk_id,
            }));

            let all_done = !self.chunks.values().any(|c| {
                c.artifact_id == input.artifact_id && self.pending_full_text.contains(&c.id)
            });

            if all_done {
                if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                    artifact.index_status = IndexStatus::Indexed;
                }
                generated.push(self.emit_event(DomainEvent::ArtifactIndexed {
                    artifact_id: input.artifact_id,
                }));
            }
        }

        Ok(generated)
    }
}
