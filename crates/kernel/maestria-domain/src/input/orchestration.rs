use std::collections::btree_map::Entry;

use crate::types::*;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

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
        generated.extend(self.process_parser_pending_artifacts(&input)?);

        // Resume/recovery path: pending_parsers survived via replay.
        generated.extend(self.process_parser_pending_parsers(&input)?);

        // pending_parsers is NOT removed here — it stays until terminal
        // ArtifactIndexed, so a crash before evidence/indexing leaves the
        // parser retryable on the next resume.

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

        let mut new_cards = 0u32;
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
                new_cards += 1;
            }
        }

        let already_parsed = self.parsed_artifact_ids.contains(&input.artifact_id);
        if new_chunks > 0 || new_cards > 0 || !already_parsed {
            let parsed = self.emit_event(DomainEvent::ArtifactParsed {
                artifact_id: input.artifact_id,
                chunks_added: new_chunks,
            });
            generated.push(parsed);
            self.parsed_artifact_ids.insert(input.artifact_id);
        }

        Ok(generated)
    }

    // ── ParserCompleted decomposition helpers ──────────────────────────────

    /// First-time commit from fresh detection (pending_artifacts).
    /// On fresh ingestion this fires once; on retry or resume
    /// the pending_artifacts entry is absent so this is skipped.
    fn process_parser_pending_artifacts(
        &mut self,
        input: &ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();
        if let Some(pending) = self.pending_artifacts.remove(&input.artifact_id) {
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
        Ok(generated)
    }

    /// Resume/recovery path: no pending_artifacts (in-memory only, lost on
    /// restart), but pending_parsers survived via replay. Ensure the artifact
    /// exists and has correct Pending status.
    fn process_parser_pending_parsers(
        &mut self,
        input: &ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();
        if let Some(parser) = self.pending_parsers.get(&input.artifact_id).cloned() {
            if let Entry::Vacant(entry) = self.artifacts.entry(input.artifact_id) {
                entry.insert(Artifact::with_title(
                    input.artifact_id,
                    parser.title.clone(),
                ));
                let register_event = self.emit_event(DomainEvent::ArtifactRegistered {
                    artifact_id: input.artifact_id,
                    title: parser.title.clone(),
                });
                generated.push(register_event);
            } else if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                // Artifact exists from replayed ArtifactRegistered. Ensure title
                // is populated — ParserStarted may carry a richer title.
                if artifact.title.is_empty() && !parser.title.is_empty() {
                    artifact.title = parser.title.clone();
                }
            }
            // Transition to Pending if not already Pending with the same hash.
            // Indexed/Unindexed states are not silently skipped.
            if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                let needs_pending = artifact.index_status != IndexStatus::Pending
                    || artifact.content_hash.as_deref() != Some(&parser.content_hash);
                if needs_pending {
                    artifact.content_hash = Some(parser.content_hash.clone());
                    artifact.index_status = IndexStatus::Pending;
                    let pending_event = self.emit_event(DomainEvent::PendingIndex {
                        artifact_id: input.artifact_id,
                        content_hash: parser.content_hash,
                    });
                    generated.push(pending_event);
                }
            }
        }
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

        Ok(generated)
    }

    pub(super) fn handle_approval_resolved(
        &mut self,
        input: ApprovalDecision,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        // Idempotency: already-resolved approvals produce no new events.
        if self.resolved_approvals.contains(&input.approval_id) {
            return Ok(vec![]);
        }

        let task = self
            .tasks
            .get(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;

        let from_status = task.status;
        let mut emitted = vec![];

        if input.approved {
            match from_status {
                TaskStatus::Draft => {
                    // Two-step domain transition: Draft→Open→Active,
                    // but emit a single authoritative event.
                    self.handle_change_task_status(input.task_id, TaskStatus::Open)?;
                    self.handle_change_task_status(input.task_id, TaskStatus::Active)?;
                    emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                        approval_id: input.approval_id,
                        task_id: input.task_id,
                        approved: true,
                        from_status: Some(TaskStatus::Draft),
                        to_status: Some(TaskStatus::Active),
                    }));
                }
                TaskStatus::Open | TaskStatus::Blocked => {
                    let to_status = TaskStatus::Active;
                    self.handle_change_task_status(input.task_id, to_status)?;
                    emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                        approval_id: input.approval_id,
                        task_id: input.task_id,
                        approved: true,
                        from_status: Some(from_status),
                        to_status: Some(to_status),
                    }));
                }
                _ => {
                    // Already in terminal state; record without transition.
                    emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                        approval_id: input.approval_id,
                        task_id: input.task_id,
                        approved: true,
                        from_status: Some(from_status),
                        to_status: Some(from_status),
                    }));
                }
            }
        } else {
            let to_status = if from_status.can_transition_to(TaskStatus::Blocked) {
                self.handle_change_task_status(input.task_id, TaskStatus::Blocked)?;
                TaskStatus::Blocked
            } else {
                from_status
            };
            emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                approval_id: input.approval_id,
                task_id: input.task_id,
                approved: false,
                from_status: Some(from_status),
                to_status: Some(to_status),
            }));
        }

        self.resolved_approvals.insert(input.approval_id);
        Ok(emitted)
    }

    // ── SearchExecuted (audit) ────────────────────────────────────

    pub(super) fn handle_search_executed(
        &mut self,
        input: SearchExecutedInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if input.query.trim().is_empty() {
            return Err(DomainError::EmptyIntent);
        }
        // Audit event: no state mutation, just record the fact.
        Ok(self.emit_event(DomainEvent::SearchExecuted {
            query: input.query,
            limit: input.limit,
            evidence_ids: input.evidence_ids,
            at: input.at,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────

    pub(crate) fn apply_user_intent_observed(
        &mut self,
        task_id: TaskId,
        title: &str,
    ) -> Result<(), DomainError> {
        if title.trim().is_empty() {
            return Err(DomainError::EmptyIntent);
        }
        if !self.tasks.contains_key(&task_id) {
            return Err(DomainError::MissingTask { id: task_id });
        }
        Ok(())
    }

    pub(crate) fn apply_search_completed(
        &mut self,
        artifact_id: ArtifactId,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        // SearchCompleted must never touch pending parser metadata.
        Ok(())
    }

    pub(crate) fn apply_harness_run_completed(
        &mut self,
        task_id: Option<TaskId>,
    ) -> Result<(), DomainError> {
        match task_id {
            Some(id) if !self.tasks.contains_key(&id) => {
                return Err(DomainError::MissingTask { id });
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn apply_approval_recorded(
        &mut self,
        approval_id: ApprovalId,
        task_id: TaskId,
        from_status: Option<TaskStatus>,
        to_status: Option<TaskStatus>,
    ) -> Result<(), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        // Legacy events (None/None) only record identity without status mutation.
        // Authoritative events (Some/Some) must have matching current status.
        match (from_status, to_status) {
            (None, None) => {}
            (Some(from), Some(to)) => {
                if task.status != from {
                    return Err(DomainError::InternalInvariantViolation {
                        detail: "approval replay: task status does not match from_status",
                    });
                }
                if from != to {
                    task.status = to;
                }
            }
            _ => {
                return Err(DomainError::InternalInvariantViolation {
                    detail: "approval replay: mixed Some/None from/to status fields",
                });
            }
        }
        self.resolved_approvals.insert(approval_id);
        Ok(())
    }

    pub(crate) fn apply_tick_observed(&mut self) {}

    pub(crate) fn apply_search_executed(&mut self, query: &str) -> Result<(), DomainError> {
        if query.trim().is_empty() {
            return Err(DomainError::EmptyIntent);
        }
        // SearchExecuted is a pure audit event — no state mutation on replay.
        Ok(())
    }
}
