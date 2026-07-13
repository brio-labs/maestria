use crate::config::EffectExecutionContext;
use maestria_domain::{ArtifactId, BlobId, DomainEvent, DomainEventEnvelope, PersistStateRequest};
use maestria_ports::{EventFilter, EventLog, PortError};
use std::time::Duration;

impl EffectExecutionContext {
    /// Persist a domain event to the event log, then cascade persistence
    /// for the associated domain entity (artifact, chunk, card, evidence).
    /// Returns `false` on any persistence failure — callers should treat
    /// this as a fatal runtime error.
    pub(crate) async fn handle_persist_event(&self, envelope: DomainEventEnvelope) -> bool {
        if !self.append_event_with_conflict_resolution(&envelope) {
            return false;
        }
        let persisted = self.persist_event_entity(&envelope.event).await;
        if persisted {
            self.ack_harness_feedback(envelope.id);
        }
        persisted
    }
    fn ack_harness_feedback(&self, event_id: maestria_domain::EventId) {
        let feedback = match self.feedback_acks.lock() {
            Ok(mut pending) => pending.remove(&event_id),
            Err(_) => {
                tracing::error!("harness feedback acknowledgement lock poisoned");
                None
            }
        };
        if let Some((run_id, generation)) = feedback
            && let Err(error) = self.adapters.effect_journal.record_terminal(
                run_id,
                generation,
                maestria_ports::EffectJournalStatus::Completed,
            )
        {
            tracing::error!(
                %error,
                %run_id,
                generation,
                "failed to finalize persisted harness feedback"
            );
        }
    }

    /// Append the envelope to the event log. On conflict, scan to verify
    /// the event already exists and is identical. Returns `false` on any
    /// persistence error.
    fn append_event_with_conflict_resolution(&self, envelope: &DomainEventEnvelope) -> bool {
        match self.adapters.event_log.append(envelope.clone()) {
            Ok(()) => true,
            Err(PortError::Conflict { .. }) => {
                match self
                    .adapters
                    .event_log
                    .scan(EventFilter { artifact_id: None })
                {
                    Ok(events) if events.iter().any(|stored| stored == envelope) => true,
                    Ok(_) => {
                        tracing::error!("event persistence conflict for a different envelope");
                        false
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to verify persisted event after conflict");
                        false
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to persist event");
                false
            }
        }
    }

    /// Cascade-persist the domain entity associated with the event:
    /// read the current entity from in-memory state, then write it
    /// through the corresponding repository. Returns `false` on any
    /// persistence failure.
    async fn persist_event_entity(&self, event: &DomainEvent) -> bool {
        match event {
            DomainEvent::ArtifactRegistered { artifact_id, .. } => {
                let artifact = {
                    let state = self.state.read().await;
                    state.artifacts.get(artifact_id).cloned()
                };
                if let Some(artifact) = artifact {
                    if let Err(error) = self.adapters.artifact_repo.put(artifact) {
                        tracing::error!(%artifact_id, %error, "failed to persist artifact");
                        return false;
                    }
                } else {
                    tracing::error!(%artifact_id, "artifact missing from state during persist");
                    return false;
                }
            }
            DomainEvent::ChunkRegistered { chunk_id, .. } => {
                let chunk = {
                    let state = self.state.read().await;
                    state.chunks.get(chunk_id).cloned()
                };
                if let Some(chunk) = chunk {
                    if let Err(error) = self.adapters.chunk_repo.put(chunk) {
                        tracing::error!(%chunk_id, %error, "failed to persist chunk");
                        return false;
                    }
                } else {
                    tracing::error!(%chunk_id, "chunk missing from state during persist");
                    return false;
                }
            }
            DomainEvent::CardCreated { card_id, .. } => {
                let card = {
                    let state = self.state.read().await;
                    state.cards.get(card_id).cloned()
                };
                if let Some(card) = card {
                    if let Err(error) = self.adapters.card_repo.put(card) {
                        tracing::error!(%card_id, %error, "failed to persist card");
                        return false;
                    }
                } else {
                    tracing::error!(%card_id, "card missing from state during persist");
                    return false;
                }
            }
            DomainEvent::EvidenceRecorded { evidence_id, .. } => {
                let evidence = {
                    let state = self.state.read().await;
                    state.evidences.get(evidence_id).cloned()
                };
                if let Some(evidence) = evidence {
                    if let Err(error) = self.adapters.evidence_repo.replace(evidence) {
                        tracing::error!(%evidence_id, %error, "failed to persist evidence");
                        return false;
                    }
                } else {
                    tracing::error!(%evidence_id, "evidence missing from state during persist");
                    return false;
                }
            }
            DomainEvent::PendingIndex { artifact_id, .. }
            | DomainEvent::ArtifactIndexed { artifact_id } => {
                let artifact = {
                    let state = self.state.read().await;
                    state.artifacts.get(artifact_id).cloned()
                };
                if let Some(artifact) = artifact {
                    if let Err(error) = self.adapters.artifact_repo.put(artifact) {
                        tracing::error!(%artifact_id, %error, "failed to persist artifact update");
                        return false;
                    }
                } else {
                    tracing::error!(%artifact_id, "artifact missing from state during index-status persist");
                    return false;
                }
            }
            _ => {}
        }
        true
    }

    /// Log a state-snapshot request. Full persistence of the kernel state
    /// is deferred to a future durability layer.
    pub(crate) async fn handle_persist_state(&self, request: PersistStateRequest) -> bool {
        let state = self.state.read().await;
        tracing::info!(
            reason = %request.reason,
            artifacts = state.artifacts.len(),
            chunks = state.chunks.len(),
            tasks = state.tasks.len(),
            events = state.event_log.len(),
            "state snapshot requested"
        );
        true
    }
}

/// Polls the event log for a persisted ParserStarted envelope matching
/// `artifact_id`, `blob_id`, _and_ `content_hash`. Returns `true` once
/// the event is observable, or `false` on timeout / scan error.
/// Uses deterministic backoff to avoid busy-waiting while the domain
/// loop commits the event.
pub(crate) async fn wait_for_parser_started_persistence(
    event_log: &dyn EventLog,
    artifact_id: ArtifactId,
    blob_id: BlobId,
    content_hash_val: &str,
    barrier_timeout: Duration,
) -> bool {
    // Scan all events — EventFilter artifact_id filtering may not cover
    // ParserStarted in every EventLog implementation.
    let contains_started = |entries: &[DomainEventEnvelope]| -> bool {
        entries.iter().any(|e| {
            matches!(
                &e.event,
                DomainEvent::ParserStarted {
                    artifact_id: id,
                    blob_id: bid,
                    content_hash: ch,
                    ..
                } if *id == artifact_id
                    && *bid == blob_id
                    && ch == content_hash_val
            )
        })
    };

    // Immediate check without sleeping.
    match event_log.scan(EventFilter { artifact_id: None }) {
        Ok(events) if contains_started(&events) => return true,
        Err(error) => {
            tracing::error!(%error, "failed to scan event log for ParserStarted barrier");
            return false;
        }
        _ => {}
    }

    let deadline = tokio::time::Instant::now() + barrier_timeout;
    let mut backoff_ms: u64 = 1;

    loop {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                artifact_id = %artifact_id,
                %blob_id,
                timeout_ms = barrier_timeout.as_millis(),
                "ParserStarted persistence barrier timed out; not parsing"
            );
            return false;
        }

        tokio::time::sleep(Duration::from_millis(backoff_ms.min(500))).await;
        backoff_ms = backoff_ms.saturating_mul(2);

        match event_log.scan(EventFilter { artifact_id: None }) {
            Ok(events) if contains_started(&events) => return true,
            Err(error) => {
                tracing::error!(%error, "failed to scan event log during ParserStarted barrier");
                return false;
            }
            _ => {}
        }
    }
}
