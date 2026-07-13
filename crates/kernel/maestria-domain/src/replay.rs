use crate::types::*;

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
            DomainEvent::ArtifactRegistered { .. }
            | DomainEvent::ChunkRegistered { .. }
            | DomainEvent::ParserStarted { .. }
            | DomainEvent::ArtifactParsed { .. }
            | DomainEvent::PendingIndex { .. }
            | DomainEvent::FullTextIndexed { .. }
            | DomainEvent::ArtifactIndexed { .. } => {
                self.replay_artifact_events(&envelope.event)?;
            }
            DomainEvent::MemoryCandidateCreated { .. }
            | DomainEvent::MemoryPromoted { .. }
            | DomainEvent::MemoryContradicted { .. }
            | DomainEvent::MemoryDeprecated { .. }
            | DomainEvent::MemorySuperseded { .. } => {
                self.replay_memory_events(&envelope.event)?;
            }
            DomainEvent::UserIntentObserved { .. }
            | DomainEvent::SearchCompleted { .. }
            | DomainEvent::SearchExecuted { .. }
            | DomainEvent::HarnessRunCompleted { .. }
            | DomainEvent::ApprovalRecorded { .. }
            | DomainEvent::TickObserved { .. } => {
                self.replay_orchestration_events(&envelope.event)?;
            }
            DomainEvent::CardCreated { .. }
            | DomainEvent::ClaimCreated { .. }
            | DomainEvent::ClaimValidationUpdated { .. }
            | DomainEvent::ClaimEvidenceLinked { .. }
            | DomainEvent::EvidenceRecorded { .. }
            | DomainEvent::TaskOpened { .. }
            | DomainEvent::TaskStatusChanged { .. }
            | DomainEvent::TaskCompletionRecorded { .. }
            | DomainEvent::TaskEvidenceLinked { .. }
            | DomainEvent::RelationCreated { .. }
            | DomainEvent::ValidationReportCreated { .. } => {
                self.replay_entity_events(&envelope.event)?;
            }
        }

        self.event_log.push(envelope);
        Ok(())
    }

    // ── Group dispatch helpers ─────────────────────────────────────────────

    fn replay_artifact_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::ArtifactRegistered { artifact_id, title } => {
                self.apply_artifact_registered(*artifact_id, title)
            }
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
                text,
            } => self.apply_chunk_registered(*chunk_id, *artifact_id, *order, text),
            DomainEvent::ParserStarted {
                artifact_id,
                title,
                source_path,
                content_hash,
                blob_id,
            } => {
                self.apply_parser_started(*artifact_id, title, source_path, content_hash, *blob_id);
                Ok(())
            }
            DomainEvent::ArtifactParsed { artifact_id, .. } => {
                self.apply_artifact_parsed(*artifact_id)
            }
            DomainEvent::PendingIndex {
                artifact_id,
                content_hash,
            } => self.apply_pending_index(*artifact_id, content_hash),
            DomainEvent::FullTextIndexed {
                artifact_id,
                chunk_id,
            } => self.apply_full_text_indexed(*artifact_id, *chunk_id),
            DomainEvent::ArtifactIndexed { artifact_id } => {
                self.apply_artifact_indexed(*artifact_id)
            }
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_artifact_events: unexpected event variant",
            }),
        }
    }

    fn replay_memory_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => self.apply_memory_candidate_created(
                *candidate_id,
                *claim_id,
                evidence_ids,
                *confidence_milli,
            ),
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
            } => self.apply_memory_promoted(*memory_id, *candidate_id),
            DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => self.apply_memory_contradicted(*memory_id, *contradicting_candidate_id),
            DomainEvent::MemoryDeprecated { memory_id } => self.apply_memory_deprecated(*memory_id),
            DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => self.apply_memory_superseded(*memory_id, *by_memory_id),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_memory_events: unexpected event variant",
            }),
        }
    }
    fn replay_orchestration_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::UserIntentObserved { task_id, title } => {
                self.apply_user_intent_observed(*task_id, title)
            }
            DomainEvent::SearchCompleted { artifact_id, .. } => {
                self.apply_search_completed(*artifact_id)
            }
            DomainEvent::SearchExecuted { query, .. } => self.apply_search_executed(query),
            DomainEvent::HarnessRunCompleted { task_id, .. } => {
                self.apply_harness_run_completed(*task_id)
            }
            DomainEvent::ApprovalRecorded {
                approval_id,
                task_id,
                ..
            } => self.apply_approval_recorded(*approval_id, *task_id),
            DomainEvent::TickObserved { .. } => {
                self.apply_tick_observed();
                Ok(())
            }
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_orchestration_events: unexpected event variant",
            }),
        }
    }

    fn replay_entity_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                title,
                body,
            } => self.apply_card_created(*card_id, *artifact_id, title, body),
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
            } => self.apply_claim_created(*claim_id, *artifact_id, text, evidence_ids),
            DomainEvent::ClaimValidationUpdated { claim_id, status } => {
                self.apply_claim_validation_updated(*claim_id, status)
            }
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => self.apply_claim_evidence_linked(*claim_id, *evidence_id),
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
                excerpt,
                observed_at,
            } => self.apply_evidence_recorded(
                *evidence_id,
                *artifact_id,
                *claim_id,
                kind,
                excerpt,
                *observed_at,
            ),
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => self.apply_task_opened(*task_id, title, *priority, *artifact_id),
            DomainEvent::TaskStatusChanged { task_id, from, to } => {
                self.apply_task_status_changed(*task_id, *from, *to)
            }
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => self.apply_task_completion_recorded(*task_id, *status, *validation_report_id),
            DomainEvent::TaskEvidenceLinked {
                task_id,
                evidence_id,
            } => self.apply_task_evidence_linked(*task_id, *evidence_id),
            DomainEvent::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => self.apply_relation_created(
                *relation_id,
                *source,
                *kind,
                *target,
                *evidence_id,
                *confidence_milli,
            ),
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => self.apply_validation_report_created(*report_id, *task_id, *passed, warnings),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_entity_events: unexpected event variant",
            }),
        }
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
