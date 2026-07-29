use crate::SearchCompatibilityError;
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

        // Keep the durable parser marker while governed OCR is outstanding.
        let ocr_pending = input.status == crate::provenance::ParseStatus::NeedsOcr
            && self
                .pending_ocr
                .values()
                .any(|intent| intent.artifact_id() == input.artifact_id);
        if input.status != crate::provenance::ParseStatus::Parsed && !ocr_pending {
            self.pending_parsers.remove(&input.artifact_id);
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
                chunks_added: new_chunks,
            });
            generated.push(parsed);
            self.parsed_artifact_ids.insert(input.artifact_id);
            if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
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
        let mut emitted = vec![];
        if !input.affects_task {
            emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                approval_id: input.approval_id,
                task_id: input.task_id,
                approved: input.approved,
                from_status: None,
                to_status: None,
            }));
            self.resolved_approvals.insert(input.approval_id);
            return Ok(emitted);
        }
        let Some(task_id) = input.task_id else {
            emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                approval_id: input.approval_id,
                task_id: None,
                approved: input.approved,
                from_status: None,
                to_status: None,
            }));
            self.resolved_approvals.insert(input.approval_id);
            return Ok(emitted);
        };

        let task = self
            .tasks
            .get(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;

        let from_status = task.status;

        if input.approved {
            match from_status {
                TaskStatus::Draft => {
                    // Two-step domain transition: Draft→Open→Active,
                    // but emit a single authoritative event.
                    self.handle_change_task_status(task_id, TaskStatus::Open)?;
                    self.handle_change_task_status(task_id, TaskStatus::Active)?;
                    emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                        approval_id: input.approval_id,
                        task_id: Some(task_id),
                        approved: true,
                        from_status: Some(TaskStatus::Draft),
                        to_status: Some(TaskStatus::Active),
                    }));
                }
                TaskStatus::Open | TaskStatus::Blocked => {
                    let to_status = TaskStatus::Active;
                    self.handle_change_task_status(task_id, to_status)?;
                    emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                        approval_id: input.approval_id,
                        task_id: Some(task_id),
                        approved: true,
                        from_status: Some(from_status),
                        to_status: Some(to_status),
                    }));
                }
                _ => {
                    emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                        approval_id: input.approval_id,
                        task_id: Some(task_id),
                        approved: true,
                        from_status: Some(from_status),
                        to_status: Some(from_status),
                    }));
                }
            }
        } else {
            let to_status = if from_status.can_transition_to(TaskStatus::Blocked) {
                self.handle_change_task_status(task_id, TaskStatus::Blocked)?;
                TaskStatus::Blocked
            } else {
                from_status
            };
            emitted.push(self.emit_event(DomainEvent::ApprovalRecorded {
                approval_id: input.approval_id,
                task_id: Some(task_id),
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
            pack_metadata: input.pack_metadata,
            at: input.at,
        }))
    }

    pub(super) fn handle_search_knowledge_completed(
        &mut self,
        input: crate::inputs::SearchKnowledgeCompleted,
    ) -> Result<DomainEventEnvelope, DomainError> {
        input
            .plan
            .validate_schema()
            .map_err(|error| DomainError::SearchIncompatible { error })?;
        input
            .outcome
            .verify_compatibility(&input.plan)
            .map_err(|error| DomainError::SearchIncompatible { error })?;
        let expected_policy = input
            .plan
            .authorization
            .as_ref()
            .ok_or(DomainError::SearchIncompatible {
                error: SearchCompatibilityError::TracePlanMismatch(
                    "authorization snapshot is missing",
                ),
            })?
            .canonical_fingerprint();
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
            Some(id) if !self.tasks.contains_key(&id) => Err(DomainError::MissingTask { id }),
            _ => Ok(()),
        }
    }

    pub(crate) fn apply_approval_recorded(
        &mut self,
        approval_id: ApprovalId,
        task_id: Option<TaskId>,
        from_status: Option<TaskStatus>,
        to_status: Option<TaskStatus>,
    ) -> Result<(), DomainError> {
        if from_status.is_none() && to_status.is_none() {
            self.resolved_approvals.insert(approval_id);
            return Ok(());
        }
        let Some(task_id) = task_id else {
            return Err(DomainError::InternalInvariantViolation {
                detail: "taskless approval event cannot carry task status transition",
            });
        };
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return Err(DomainError::MissingTask { id: task_id });
        };
        match (from_status, to_status) {
            (None, None) => {}
            (Some(from), Some(to)) => {
                if task.status != from {
                    return Err(DomainError::InternalInvariantViolation {
                        detail: "approval replay: task status does not match from_status",
                    });
                }
                if from != to {
                    let valid = (from == TaskStatus::Draft && to == TaskStatus::Active)
                        || from.can_transition_to(to);
                    if !valid {
                        return Err(DomainError::InternalInvariantViolation {
                            detail: "approval replay: invalid status transition in ApprovalRecorded",
                        });
                    }
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

    pub(crate) fn apply_search_knowledge_completed(&mut self) -> Result<(), DomainError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_approval_retains_task_audit_without_transition() -> Result<(), DomainError> {
        let task_id = TaskId::new(7);
        let mut state = KernelState::new();
        state.tasks.insert(
            task_id,
            Task::new(task_id, "task".into(), TaskPriority::High),
        );
        let output = state.apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
            approval_id: ApprovalId::new(9),
            task_id: Some(task_id),
            approved: true,
            affects_task: false,
        }))?;
        assert_eq!(state.tasks[&task_id].status, TaskStatus::Draft);
        assert!(matches!(
            output.events[0].event,
            DomainEvent::ApprovalRecorded {
                task_id: Some(id),
                from_status: None,
                to_status: None,
                ..
            } if id == task_id
        ));
        Ok(())
    }
}
