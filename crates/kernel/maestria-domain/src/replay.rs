use crate::types::*;

mod notebook;
mod ocr;
#[path = "replay_dispatch.rs"]
mod replay_dispatch;
mod replay_entities;
mod source;
impl KernelState {
    pub fn apply_event(&mut self, envelope: DomainEventEnvelope) -> Result<(), DomainError> {
        let expected_id = self.event_log.len() as u64 + 1;
        if envelope.id.value() != expected_id {
            return Err(DomainError::InvalidEventId {
                expected: expected_id,
                actual: envelope.id.value(),
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
            DomainEvent::SearchCompleted { .. }
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
            DomainEvent::SourceBecameStale {
                artifact_id,
                source_path,
                ..
            } => self.replay_source_became_stale(*artifact_id, source_path),
            DomainEvent::NotebookCreated { .. }
            | DomainEvent::NotebookRenamed { .. }
            | DomainEvent::NotebookDeleted { .. }
            | DomainEvent::NotebookSourceAttached { .. }
            | DomainEvent::NotebookSourceDetached { .. }
            | DomainEvent::NotebookDraftSaved { .. }
            | DomainEvent::NotebookDraftDeleted { .. } => {
                self.replay_notebook_event(&envelope.event)?;
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
