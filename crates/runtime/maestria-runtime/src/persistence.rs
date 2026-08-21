use crate::config::EffectExecutionContext;
use maestria_domain::{DomainEvent, DomainEventEnvelope, KernelState};
use maestria_ports::{EventFilter, PortError, RealmReadGrantRepository};
use std::collections::BTreeSet;

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
        if persisted && !self.ack_harness_feedback(envelope.id) {
            return false;
        }
        persisted
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
        let Some(entity) = entity else {
            Self::log_missing_persist_entity(id, entity_name, context);
            return false;
        };
        if let Err(error) = put(entity) {
            Self::log_persist_error(id, error, entity_name, context);
            return false;
        }
        true
    }

    fn log_missing_persist_entity<Id: std::fmt::Display>(
        id: Id,
        entity_name: &'static str,
        context: Option<&'static str>,
    ) {
        if let Some(ctx) = context {
            tracing::error!(%id, "{entity_name} missing from state during {ctx} persist");
        } else {
            tracing::error!(%id, "{entity_name} missing from state during persist");
        }
    }

    fn log_persist_error<Id: std::fmt::Display>(
        id: Id,
        error: PortError,
        entity_name: &'static str,
        context: Option<&'static str>,
    ) {
        if let Some(ctx) = context {
            tracing::error!(%id, %error, "failed to persist {entity_name} {ctx}");
        } else {
            tracing::error!(%id, %error, "failed to persist {entity_name}");
        }
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
            DomainEvent::RealmReadGrantIssued { grant } => {
                let token_digest = grant.token_digest().clone();
                self.persist_entity(
                    token_digest.clone(),
                    |state| state.realm_read_grants.get(&token_digest).cloned(),
                    |grant| self.adapters.realm_read_grant_repo.put(grant),
                    "realm read grant",
                    Some("issue"),
                )
                .await
            }
            DomainEvent::RealmReadGrantRevoked { token_digest } => {
                self.persist_entity(
                    token_digest.clone(),
                    |state| state.realm_read_grants.get(token_digest).cloned(),
                    |grant| self.adapters.realm_read_grant_repo.put(grant),
                    "realm read grant",
                    Some("revoke"),
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
            DomainEvent::ApprovalRecorded {
                approval_id,
                outcome,
            } => match self
                .adapters
                .approval_repo
                .resolve(*approval_id, outcome.approved())
            {
                Ok(Some(_)) => true,
                Ok(None) => {
                    tracing::error!(
                        %approval_id,
                        "approval resolution skipped: no pending record exists for the event"
                    );
                    false
                }
                Err(error) => {
                    tracing::error!(%approval_id, %error, "failed to persist approval resolution");
                    false
                }
            },
            _ => true,
        }
    }
}

/// Rebuilds the durable current-grant projection from replayed event state.
/// This is intentionally an adapter operation outside API handlers.
pub fn rebuild_realm_read_grant_projection(
    repository: &(dyn RealmReadGrantRepository + Send + Sync),
    state: &KernelState,
) -> Result<(), PortError> {
    let token_digests = state
        .realm_read_grants
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    repository.delete_not_in(&token_digests)?;
    for grant in state.realm_read_grants.values().cloned() {
        repository.put(grant)?;
    }
    Ok(())
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
