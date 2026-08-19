use std::sync::{Arc, Mutex};

use super::store::lock_map;
use crate::{EventFilter, PortError};
use maestria_domain::DomainEventEnvelope;

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
        let mut guard = lock_map(&self.events, "event log lock poisoned")?;
        let expected_id = guard.len() as u64 + 1;
        if event.id.value() != expected_id {
            return Err(PortError::Conflict {
                message: format!(
                    "expected event id {expected_id}, got id {}",
                    event.id.value()
                ),
            });
        }
        guard.push(event);
        Ok(())
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let guard = lock_map(&self.events, "event log lock poisoned")?;
        let mut entries = guard.clone();
        if let Some(artifact_id) = filter.artifact_id {
            entries.retain(|entry| entry.event.artifact_id() == Some(artifact_id));
        }
        Ok(entries)
    }
}
