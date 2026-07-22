use crate::config::EffectExecutionContext;
use maestria_domain::{
    ArtifactId, BlobId, DomainEvent, DomainEventEnvelope, KernelState, PersistStateRequest,
};
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

    /// Generic helper to read an entity from in-memory state and write it
    /// through the corresponding repository. Returns `false` on any
    /// persistence failure.
    async fn persist_entity<T, Id>(
        &self,
        id: Id,
        get: impl FnOnce(&KernelState) -> Option<T>,
        put: impl FnOnce(T) -> Result<(), PortError>,
        entity_name: &'static str,
        context: Option<&'static str>,
    ) -> bool
    where
        Id: std::fmt::Display,
    {
        let entity = {
            let state = self.state.read().await;
            get(&state)
        };
        if let Some(entity) = entity {
            if let Err(error) = put(entity) {
                if let Some(ctx) = context {
                    tracing::error!(%id, %error, "failed to persist {entity_name} {ctx}");
                } else {
                    tracing::error!(%id, %error, "failed to persist {entity_name}");
                }
                return false;
            }
        } else {
            if let Some(ctx) = context {
                tracing::error!(%id, "{entity_name} missing from state during {ctx} persist");
            } else {
                tracing::error!(%id, "{entity_name} missing from state during persist");
            }
            return false;
        }
        true
    }

    /// Cascade-persist the domain entity associated with the event:
    /// read the current entity from in-memory state, then write it
    /// through the corresponding repository. Returns `false` on any
    /// persistence failure.
    async fn persist_event_entity(&self, event: &DomainEvent) -> bool {
        match event {
            DomainEvent::ArtifactRegistered { artifact_id, .. } => {
                self.persist_entity(
                    *artifact_id,
                    |s| s.artifacts.get(artifact_id).cloned(),
                    |a| self.adapters.artifact_repo.put(a),
                    "artifact",
                    None,
                )
                .await
            }
            DomainEvent::ChunkRegistered { chunk_id, .. } => {
                self.persist_entity(
                    *chunk_id,
                    |s| s.chunks.get(chunk_id).cloned(),
                    |c| self.adapters.chunk_repo.put(c),
                    "chunk",
                    None,
                )
                .await
            }
            DomainEvent::CardCreated { card_id, .. } => {
                self.persist_entity(
                    *card_id,
                    |s| s.cards.get(card_id).cloned(),
                    |c| self.adapters.card_repo.put(c),
                    "card",
                    None,
                )
                .await
            }
            DomainEvent::EvidenceRecorded { evidence_id, .. } => {
                self.persist_entity(
                    *evidence_id,
                    |s| s.evidences.get(evidence_id).cloned(),
                    |e| self.adapters.evidence_repo.replace(e),
                    "evidence",
                    None,
                )
                .await
            }
            DomainEvent::PendingIndex { artifact_id, .. }
            | DomainEvent::ArtifactParsed { artifact_id, .. }
            | DomainEvent::ArtifactIndexed { artifact_id } => {
                self.persist_entity(
                    *artifact_id,
                    |s| s.artifacts.get(artifact_id).cloned(),
                    |a| self.adapters.artifact_repo.put(a),
                    "artifact",
                    Some("index-status"),
                )
                .await
            }
            _ => true,
        }
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
    let check_loop = async {
        let mut backoff_ms: u64 = 1;
        loop {
            tokio::time::sleep(Duration::from_millis(backoff_ms.min(500))).await;
            backoff_ms = backoff_ms.saturating_mul(2);

            match event_log.scan(EventFilter { artifact_id: None }) {
                Ok(events) if contains_started(&events) => return true,
                Err(error) => {
                    tracing::error!(%error, "Failed to scan event log in persistence barrier");
                    return false;
                }
                _ => {}
            }
        }
    };

    match tokio::time::timeout(barrier_timeout, check_loop).await {
        Ok(success) => success,
        Err(_) => {
            tracing::warn!(
                artifact_id = %artifact_id,
                %blob_id,
                timeout_ms = barrier_timeout.as_millis(),
                "ParserStarted persistence barrier timed out; not parsing"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Writer that copies every byte into a shared `Vec<u8>` so tests can
    /// assert on the exact log output produced by `tracing` macros.
    #[derive(Clone)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Build a minimal `EffectExecutionContext` backed by the default in-memory
    /// adapters and the supplied `KernelState`.
    fn test_context(state: KernelState) -> EffectExecutionContext {
        let adapters = Arc::new(test_helpers::test_adapters());
        let governance = Arc::new(test_helpers::test_governance());
        let (input_tx, _input_rx) = mpsc::channel(8);
        EffectExecutionContext::test_default(
            adapters,
            governance,
            Arc::new(tokio::sync::RwLock::new(state)),
            input_tx,
        )
    }

    #[test]
    fn persist_entity_put_succeeds_returns_true() {
        let ctx = test_context(KernelState::new());
        let result = tokio_test::block_on(async {
            ctx.persist_entity(
                42,
                |_state: &KernelState| Some("entity".to_string()),
                |_entity: String| Ok(()),
                "test-entity",
                None,
            )
            .await
        });
        assert!(result);
    }

    #[test]
    fn persist_entity_put_fails_returns_false_and_logs_error() -> Result<(), &'static str> {
        let ctx = test_context(KernelState::new());
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = CaptureWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();

        let result = tracing::subscriber::with_default(subscriber, || {
            tokio_test::block_on(async {
                ctx.persist_entity(
                    42,
                    |_state: &KernelState| Some("entity".to_string()),
                    |_entity: String| {
                        Err(PortError::Conflict {
                            message: "test".into(),
                        })
                    },
                    "test-entity",
                    None,
                )
                .await
            })
        });

        assert!(!result);
        let output = String::from_utf8(buf.lock().clone())
            .map_err(|_| "captured logs should be valid UTF-8")?;
        assert!(
            output.contains("failed to persist test-entity"),
            "expected error log about failed persist, got: {output}"
        );
        Ok(())
    }

    #[test]
    fn persist_entity_get_none_returns_false_and_logs_error() -> Result<(), &'static str> {
        let ctx = test_context(KernelState::new());
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = CaptureWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();

        let result = tracing::subscriber::with_default(subscriber, || {
            tokio_test::block_on(async {
                ctx.persist_entity(
                    42,
                    |_state: &KernelState| None::<String>,
                    |_entity: String| Ok(()),
                    "test-entity",
                    None,
                )
                .await
            })
        });

        assert!(!result);
        let output = String::from_utf8(buf.lock().clone())
            .map_err(|_| "captured logs should be valid UTF-8")?;
        assert!(
            output.contains("test-entity missing from state during persist"),
            "expected error log about missing entity, got: {output}"
        );
        Ok(())
    }

    #[test]
    fn persist_entity_context_in_error_message() -> Result<(), &'static str> {
        let ctx = test_context(KernelState::new());
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = CaptureWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();

        let result = tracing::subscriber::with_default(subscriber, || {
            tokio_test::block_on(async {
                ctx.persist_entity(
                    42,
                    |_state: &KernelState| None::<String>,
                    |_entity: String| Ok(()),
                    "test-entity",
                    Some("index-status"),
                )
                .await
            })
        });

        assert!(!result);
        let output = String::from_utf8(buf.lock().clone())
            .map_err(|_| "captured logs should be valid UTF-8")?;
        assert!(
            output.contains("index-status"),
            "expected error log to contain context 'index-status', got: {output}"
        );
        Ok(())
    }
}
