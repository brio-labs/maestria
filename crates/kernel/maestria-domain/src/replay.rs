use crate::types::*;

mod ocr;
mod replay_entities;

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
            | DomainEvent::DocumentTreeCaptured { .. }
            | DomainEvent::PendingIndex { .. }
            | DomainEvent::FullTextIndexed { .. }
            | DomainEvent::ArtifactIndexed { .. }
            | DomainEvent::OcrRequested { .. }
            | DomainEvent::OcrCompleted { .. }
            | DomainEvent::OcrFailed { .. } => {
                self.replay_artifact_events(&envelope.event)?;
            }
            DomainEvent::IndexGenerationStarted { .. }
            | DomainEvent::IndexGenerationTransitioned { .. } => {
                self.replay_generation_events(&envelope.event)?;
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
            | DomainEvent::SearchKnowledgeCompleted { .. }
            | DomainEvent::HarnessRunCompleted { .. }
            | DomainEvent::ModelAgentProposalRequested { .. }
            | DomainEvent::ModelAgentProposalCompleted { .. }
            | DomainEvent::ApprovalRecorded { .. }
            | DomainEvent::TickObserved { .. } => {
                self.replay_orchestration_events(&envelope.event)?;
            }
            DomainEvent::CardCreated { .. } => self.replay_card_events(&envelope.event)?,
            DomainEvent::ClaimCreated { .. }
            | DomainEvent::ClaimValidationUpdated { .. }
            | DomainEvent::ClaimEvidenceLinked { .. } => {
                self.replay_claim_events(&envelope.event)?
            }
            DomainEvent::EvidenceRecorded { .. } => self.replay_evidence_events(&envelope.event)?,
            DomainEvent::TaskOpened { .. }
            | DomainEvent::TaskStatusChanged { .. }
            | DomainEvent::TaskCompletionRecorded { .. }
            | DomainEvent::TaskEvidenceLinked { .. } => self.replay_task_events(&envelope.event)?,
            DomainEvent::RelationCreated { .. } => self.replay_relation_events(&envelope.event)?,
            DomainEvent::ValidationReportCreated { .. } => {
                self.replay_validation_events(&envelope.event)?;
            }
            DomainEvent::SourceBecameStale { source_path, .. } => {
                self.stale_sources.insert(source_path.clone());
            }
            DomainEvent::RealmReadGrantIssued { .. }
            | DomainEvent::RealmReadGrantRevoked { .. }
            | DomainEvent::FederatedReadAccessRecorded { .. } => {
                self.replay_federation_events(&envelope.event)?;
            }
        }

        self.event_log.push(envelope);
        Ok(())
    }

    // ── Group dispatch helpers ─────────────────────────────────────────────

    fn replay_artifact_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::ArtifactRegistered {
                artifact_id,
                title,
                security,
            } => self.apply_artifact_registered(*artifact_id, title, security),
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                node_id,
                source_span,
                representations,
                order,
                text,
            } => self.apply_chunk_registered(RegisterChunkInput {
                chunk_id: *chunk_id,
                artifact_id: *artifact_id,
                node_id: *node_id,
                source_span: *source_span,
                representations: representations.clone(),
                order: *order,
                text: text.clone(),
            }),
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
            DomainEvent::ArtifactParsed {
                artifact_id,
                status,
                ..
            } => self.apply_artifact_parsed(*artifact_id, *status),
            DomainEvent::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id,
                content_hash,
                root_id,
                nodes,
            } => self.apply_document_tree_captured(
                *artifact_id,
                *artifact_version_id,
                content_hash.clone(),
                *root_id,
                nodes,
            ),
            DomainEvent::PendingIndex {
                artifact_id,
                content_hash,
            } => self.apply_pending_index(*artifact_id, content_hash),
            DomainEvent::FullTextIndexed {
                artifact_id,
                chunk_id,
            } => self.apply_full_text_indexed(*artifact_id, *chunk_id),
            DomainEvent::OcrRequested { intent } => self.replay_ocr_requested(intent),
            DomainEvent::OcrCompleted {
                artifact_id,
                completion,
            } => self.replay_ocr_completed(*artifact_id, completion),
            DomainEvent::OcrFailed {
                artifact_id,
                request_id,
                reason,
            } => self.replay_ocr_failed(*artifact_id, request_id, reason),
            DomainEvent::ArtifactIndexed { artifact_id } => {
                self.apply_artifact_indexed(*artifact_id)
            }
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_artifact_events: unexpected event variant",
            }),
        }
    }
    fn replay_generation_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::IndexGenerationStarted {
                id,
                name,
                fingerprint,
                corpus_snapshot,
            } => self.apply_index_generation_started(
                *id,
                name.clone(),
                *corpus_snapshot,
                fingerprint.clone(),
            ),
            DomainEvent::IndexGenerationTransitioned {
                id,
                from,
                to,
                replaced_active_id,
            } => self
                .apply_index_generation_transitioned(*id, *from, *to, *replaced_active_id)
                .map(|_| ()),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_generation_events: unexpected event variant",
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
                security,
            } => self.apply_memory_candidate_created(
                *candidate_id,
                *claim_id,
                evidence_ids,
                *confidence_milli,
                security,
            ),
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
                security,
            } => self.apply_memory_promoted(*memory_id, *candidate_id, security),
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
            DomainEvent::SearchKnowledgeCompleted { .. } => self.apply_search_knowledge_completed(),
            DomainEvent::HarnessRunCompleted { task_id, .. } => {
                self.apply_harness_run_completed(*task_id)
            }
            DomainEvent::ModelAgentProposalRequested { request } => {
                if !matches!(
                    request.execution,
                    crate::model_agent::ModelAgentProposalExecution::Fresh
                ) {
                    return Err(DomainError::ModelAgentProposalRequestNotFresh {
                        run_id: request.run_id,
                    });
                }
                if self.model_agent_requests.contains_key(&request.run_id)
                    || self.model_agent_results.contains_key(&request.run_id)
                {
                    return Err(DomainError::DuplicateModelAgentProposalRunId {
                        run_id: request.run_id,
                    });
                }
                self.model_agent_requests
                    .insert(request.run_id, request.clone());
                Ok(())
            }
            DomainEvent::ModelAgentProposalCompleted { result } => {
                let run_id = result.run_id();
                if self.model_agent_results.contains_key(&run_id) {
                    return Err(DomainError::DuplicateModelAgentProposalRunId { run_id });
                }
                self.model_agent_requests.remove(&run_id);
                self.model_agent_results.insert(run_id, result.clone());
                Ok(())
            }
            DomainEvent::ApprovalRecorded {
                approval_id,
                outcome,
            } => self.apply_approval_recorded(*approval_id, *outcome),
            DomainEvent::TickObserved { .. } => {
                self.apply_tick_observed();
                Ok(())
            }
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_orchestration_events: unexpected event variant",
            }),
        }
    }

    fn replay_federation_events(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::RealmReadGrantIssued { grant } => {
                self.apply_realm_read_grant_issued(grant)
            }
            DomainEvent::RealmReadGrantRevoked { token_digest } => {
                self.apply_realm_read_grant_revoked(token_digest)
            }
            DomainEvent::FederatedReadAccessRecorded {
                token_digest,
                provider_realm,
                consumer_realm,
                record,
            } => self.apply_federated_access_recorded(
                token_digest,
                provider_realm,
                consumer_realm,
                record,
            ),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_federation_events: unexpected event variant",
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
