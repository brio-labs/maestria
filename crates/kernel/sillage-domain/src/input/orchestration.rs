use crate::SearchCompatibilityError;
use crate::types::*;
use std::sync::Arc;

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
            title: input.title,
            priority: input.priority,
            artifact_id: None,
        })?;

        Ok(vec![open])
    }

    pub(super) fn handle_parser_completed(
        &mut self,
        input: ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut staged = self.clone();
        let generated = staged.apply_parser_completed(input)?;
        *self = staged;
        Ok(generated)
    }

    fn apply_parser_completed(
        &mut self,
        input: ParserResult,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        let mut generated = Vec::new();

        // First-time commit from fresh detection (pending_artifacts).
        generated.extend(self.process_parser_pending_artifacts(&input)?);

        // Resume/recovery path: pending_parsers survived via replay.
        generated.extend(self.process_parser_pending_parsers(&input)?);

        // Keep the durable parser marker while governed OCR is outstanding.
        let ocr_pending = input.status == crate::provenance::ParseStatus::NeedsOcr
            && self
                .pending_ocr
                .values()
                .any(|intent| intent.artifact_id() == input.artifact_id);
        if input.status != crate::provenance::ParseStatus::Parsed && !ocr_pending {
            Arc::make_mut(&mut self.pending_parsers).remove(&input.artifact_id);
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let (new_chunks, new_cards) = self.register_parser_records(&input, &mut generated)?;

        let status_changed = self
            .artifacts
            .get(&input.artifact_id)
            .and_then(|artifact| artifact.parse_status)
            != Some(input.status);
        let already_parsed = self.parsed_artifact_ids.contains(&input.artifact_id);
        if new_chunks > 0 || new_cards > 0 || !already_parsed || status_changed {
            let parsed = self.emit_event(DomainEvent::ArtifactParsed {
                artifact_id: input.artifact_id,
                status: input.status,
            });
            generated.push(parsed);
            Arc::make_mut(&mut self.parsed_artifact_ids).insert(input.artifact_id);
            if let Some(artifact) = Arc::make_mut(&mut self.artifacts).get_mut(&input.artifact_id) {
                artifact.parse_status = Some(input.status);
                if input.status != crate::provenance::ParseStatus::Parsed {
                    artifact.index_status = IndexStatus::Unindexed;
                }
            }
        }

        self.capture_parser_tree(&input, &mut generated)?;
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

        generated.push(self.emit_event(DomainEvent::SearchCompleted {
            artifact_id: input.artifact_id,
        }));
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
        if self.resolved_approvals.contains(&input.approval_id()) {
            return Ok(vec![]);
        }
        let mut emitted = vec![];
        let (approval_id, task_id, approved) = match input {
            ApprovalDecision::Acknowledge {
                approval_id,
                task_id,
                approved,
            } => {
                emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                    approval_id,
                    outcome: ApprovalOutcome::Acknowledged { task_id, approved },
                }));
                Arc::make_mut(&mut self.resolved_approvals).insert(approval_id);
                return Ok(emitted);
            }
            ApprovalDecision::Resolve {
                approval_id,
                task_id,
                approved,
            } => (approval_id, task_id, approved),
        };

        let from_status = if approved {
            self.transition_to_active(task_id)?
        } else {
            self.transition_to_blocked(task_id)?
        };
        let to_status = self
            .tasks
            .get(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?
            .status;
        emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
            approval_id,
            outcome: ApprovalOutcome::TaskTransition {
                task_id,
                approved,
                from_status,
                to_status,
            },
        }));

        Arc::make_mut(&mut self.resolved_approvals).insert(approval_id);
        Ok(emitted)
    }

    /// Transition a resolved-approved task to `Active`, returning the status it
    /// started from. Draft tasks move through the two-step Draft→Open→Active
    /// path; Open/Blocked tasks transition directly; other statuses are left
    /// untouched (the transition is reported as from==to by the caller).
    fn transition_to_active(&mut self, task_id: TaskId) -> Result<TaskStatus, DomainError> {
        let from_status = self
            .tasks
            .get(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?
            .status;
        match from_status {
            TaskStatus::Draft => {
                // Two-step domain transition: Draft→Open→Active,
                // but emit a single authoritative event.
                self.handle_change_task_status(task_id, TaskStatus::Open)?;
                self.handle_change_task_status(task_id, TaskStatus::Active)?;
            }
            TaskStatus::Open | TaskStatus::Blocked => {
                self.handle_change_task_status(task_id, TaskStatus::Active)?;
            }
            _ => {}
        }
        Ok(from_status)
    }

    /// Transition a resolved-denied task to `Blocked` when its current status
    /// allows it, returning the status it started from.
    fn transition_to_blocked(&mut self, task_id: TaskId) -> Result<TaskStatus, DomainError> {
        let from_status = self
            .tasks
            .get(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?
            .status;
        if from_status.can_transition_to(TaskStatus::Blocked) {
            self.handle_change_task_status(task_id, TaskStatus::Blocked)?;
        }
        Ok(from_status)
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
            pack_metadata: input.pack_metadata,
            at: input.at,
        }))
    }

    pub(super) fn handle_search_knowledge_completed(
        &mut self,
        input: crate::inputs::SearchKnowledgeCompleted,
    ) -> Result<DomainEventEnvelope, DomainError> {
        // The plan type enforces its schema invariants at construction and
        // decode; this handler checks trace compatibility only.
        input
            .outcome
            .verify_compatibility(&input.plan)
            .map_err(|error| DomainError::SearchIncompatible { error })?;
        let expected_policy = input.plan.authorization().canonical_fingerprint();
        let found_policy = input
            .outcome
            .trace_data
            .as_ref()
            .and_then(|trace| trace.policy_fingerprint.as_deref());
        if found_policy != Some(expected_policy.as_str()) {
            return Err(DomainError::SearchIncompatible {
                error: SearchCompatibilityError::TracePlanMismatch(
                    "authorization policy differs from trusted plan snapshot",
                ),
            });
        }
        Ok(self.emit_event(DomainEvent::SearchKnowledgeCompleted {
            task_id: input.task_id,
            plan: Some(input.plan),
            outcome: input.outcome,
        }))
    }
    pub(super) fn process_retrieval_events_retired(
        &mut self,
        input: crate::inputs::RetrievalEventsRetired,
    ) -> Result<KernelOutput, DomainError> {
        // The marker's reason is the durable record of who narrowed the
        // audit trail and why (ADR-0009); an empty reason carries no
        // accountability.
        if input.reason.trim().is_empty() {
            return Err(DomainError::EmptyRetirementReason);
        }
        // Markers only advance: a request below the current high-water is
        // recorded but changes nothing.
        self.retrieval_retired_through = self.retrieval_retired_through.max(input.before_sequence);
        let envelope = self.emit_event(DomainEvent::RetrievalEventsRetired {
            before_sequence: input.before_sequence,
            reason: input.reason,
        });
        Ok(Self::output_for_event(envelope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_approval_retains_task_audit_without_transition() -> Result<(), DomainError> {
        let task_id = TaskId::new(7);
        let mut state = KernelState::new();
        Arc::make_mut(&mut state.tasks).insert(
            task_id,
            Task::new(task_id, "task".into(), TaskPriority::High),
        );
        let output = state.apply_input(DomainInput::ApprovalResolved(
            ApprovalDecision::Acknowledge {
                approval_id: ApprovalId::new(9),
                task_id: Some(task_id),
                approved: true,
            },
        ))?;
        assert_eq!(state.tasks[&task_id].status, TaskStatus::Draft);
        assert!(matches!(
            output.events[0].event,
            DomainEvent::ApprovalRecorded {
                outcome: ApprovalOutcome::Acknowledged {
                    task_id: Some(id),
                    approved: true,
                },
                ..
            } if id == task_id
        ));
        Ok(())
    }

    #[test]
    fn approval_transition_replays_to_active() -> Result<(), DomainError> {
        let task_id = TaskId::new(7);
        let mut state = KernelState::new();
        Arc::make_mut(&mut state.tasks).insert(
            task_id,
            Task::new(task_id, "task".into(), TaskPriority::High),
        );
        state.apply_approval_recorded(
            ApprovalId::new(9),
            ApprovalOutcome::TaskTransition {
                task_id,
                approved: true,
                from_status: TaskStatus::Draft,
                to_status: TaskStatus::Active,
            },
        )?;
        assert_eq!(state.tasks[&task_id].status, TaskStatus::Active);
        assert!(state.resolved_approvals.contains(&ApprovalId::new(9)));
        Ok(())
    }

    #[test]
    fn approval_replay_rejects_mismatched_from_status() -> Result<(), DomainError> {
        let task_id = TaskId::new(7);
        let mut state = KernelState::new();
        Arc::make_mut(&mut state.tasks).insert(
            task_id,
            Task::new(task_id, "task".into(), TaskPriority::High),
        );
        let error = state.apply_approval_recorded(
            ApprovalId::new(9),
            ApprovalOutcome::TaskTransition {
                task_id,
                approved: true,
                from_status: TaskStatus::Active,
                to_status: TaskStatus::Blocked,
            },
        );
        assert!(matches!(
            error,
            Err(DomainError::InternalInvariantViolation { .. })
        ));
        Ok(())
    }
}
