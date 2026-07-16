use std::sync::{Arc, Mutex};

use crate::{DomainEvent, DomainEventEnvelope, EventFilter, PortError};

#[derive(Clone, Default)]
pub struct InMemoryEventLog {
    events: Arc<Mutex<Vec<DomainEventEnvelope>>>,
}

impl InMemoryEventLog {
    pub fn new() -> Self {
        Self::default()
    }
}

impl crate::EventLog for InMemoryEventLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let mut guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        let expected_sequence = guard.len() as u64 + 1;
        if event.sequence.value() != expected_sequence || event.id.value() != expected_sequence {
            return Err(PortError::Conflict {
                message: format!(
                    "expected sequence/id {}, got seq {}, id {}",
                    expected_sequence,
                    event.sequence.value(),
                    event.id.value()
                ),
            });
        }
        guard.push(event);
        Ok(())
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        let mut entries = guard.clone();
        if let Some(artifact_id) = filter.artifact_id {
            entries.retain(|entry| match &entry.event {
                DomainEvent::ArtifactRegistered {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ChunkRegistered {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::CardCreated {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ClaimCreated {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::EvidenceRecorded {
                    artifact_id: current,
                    ..
                } => *current == artifact_id,
                DomainEvent::TaskOpened {
                    artifact_id: Some(current),
                    ..
                }
                | DomainEvent::ArtifactParsed {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::DocumentTreeCaptured {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::SearchCompleted {
                    artifact_id: current,
                    ..
                } => *current == artifact_id,
                _ => false,
            });
        }
        Ok(entries)
    }
}
