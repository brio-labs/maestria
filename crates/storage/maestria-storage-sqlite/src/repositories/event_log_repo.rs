use maestria_domain::{DomainEvent, DomainEventEnvelope, SearchTraceId};
use maestria_ports::{EventFilter, EventLog, PortError};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeMap;

use crate::{
    events::{StoredEvent, read_stored_event},
    sqlite_store::{i64_to_u64, map_append_error, optional_u64_to_i64, to_port_error, u64_to_i64},
};
fn decode_scanned_events(stored: Vec<StoredEvent>) -> Result<Vec<DomainEventEnvelope>, PortError> {
    let mut envelopes = Vec::with_capacity(stored.len());
    let mut trace_remap = BTreeMap::<SearchTraceId, SearchTraceId>::new();
    let mut search_executed_indices = Vec::new();

    for (index, event) in stored.into_iter().enumerate() {
        let (envelope, remap) = event.into_domain_with_trace_remap()?;
        if let Some((old_trace, new_trace)) = remap
            && let Some(previous) = trace_remap.insert(old_trace, new_trace)
            && previous != new_trace
        {
            return Err(PortError::Conflict {
                message: format!(
                    "legacy search trace maps to conflicting canonical identities: {old_trace}"
                ),
            });
        }
        if let DomainEvent::SearchExecuted {
            pack_metadata: Some(metadata),
            ..
        } = &envelope.event
            && matches!(
                metadata.reproducibility,
                maestria_domain::EvidencePackReproducibilityRecord::Frozen(_)
            )
        {
            search_executed_indices.push(index);
        }
        envelopes.push(envelope);
    }

    if !trace_remap.is_empty() {
        for index in search_executed_indices {
            if let DomainEvent::SearchExecuted {
                pack_metadata: Some(metadata),
                ..
            } = &mut envelopes[index].event
                && let maestria_domain::EvidencePackReproducibilityRecord::Frozen(replay) =
                    &mut metadata.reproducibility
                && let Some(replacement) = trace_remap.get(&replay.trace)
            {
                replay.trace = *replacement;
            }
        }
    }

    Ok(envelopes)
}

impl EventLog for crate::SqliteStore {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let record = StoredEvent::from_domain(&event)?;
        self.with_transaction(|transaction| {
            let last_id: Option<i64> = transaction
                .query_row(
                    "SELECT id FROM domain_events ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(to_port_error)?
                .flatten();
            let count = match last_id {
                Some(id) if id > 0 => i64_to_u64(id)?,
                Some(_) => {
                    return Err(PortError::Conflict {
                        message: "stored event log has invalid ids".to_string(),
                    });
                }
                None => 0,
            };
            let expected_id = count + 1;
            if record.id != expected_id {
                return Err(PortError::Conflict {
                    message: format!("expected event id {expected_id}, got id {}", record.id),
                });
            }
            transaction
                .execute(
                    "INSERT INTO domain_events \
                         (id, event_kind, artifact_id, payload_json, payload_version)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        u64_to_i64(record.id)?,
                        record.kind,
                        optional_u64_to_i64(record.artifact_id)?,
                        record.payload_json,
                        record.payload_version,
                    ],
                )
                .map_err(map_append_error)?;
            Ok(())
        })
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let connection = self.lock()?;
        let mut stored = Vec::new();

        if let Some(artifact_id) = filter.artifact_id {
            let mut statement = connection
                .prepare_cached(
                    "SELECT e.id, e.event_kind, e.artifact_id, e.payload_json,
                    e.payload_version
             FROM domain_events e
             WHERE e.artifact_id = ?1
             ORDER BY e.id ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement
                .query(params![u64_to_i64(artifact_id.value())?])
                .map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                stored.push(read_stored_event(row)?);
            }
        } else {
            let mut statement = connection
                .prepare_cached(
                    "SELECT e.id, e.event_kind, e.artifact_id, e.payload_json,
                    e.payload_version
             FROM domain_events e
             WHERE NOT (e.id < COALESCE((SELECT MAX(id) FROM domain_events
                 WHERE event_kind = 'retrieval_events_retired'), 0)
                 AND e.event_kind IN ('search_executed', 'search_knowledge_completed'))
             ORDER BY e.id ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement.query([]).map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                stored.push(read_stored_event(row)?);
            }
        }

        decode_scanned_events(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{EventId, LogicalTick};

    fn envelope(id: u64, event: DomainEvent) -> DomainEventEnvelope {
        DomainEventEnvelope {
            id: EventId::new(id),
            event,
        }
    }

    fn search_executed(query: &str) -> DomainEvent {
        DomainEvent::SearchExecuted {
            query: query.to_string(),
            limit: 5,
            evidence_ids: Vec::new(),
            pack_metadata: None,
            at: LogicalTick::new(1),
        }
    }

    #[test]
    fn scan_skips_retrieval_audit_events_below_marker() -> Result<(), PortError> {
        let store = crate::SqliteStore::open(":memory:")?;
        EventLog::append(&store, envelope(1, search_executed("retired query")))?;
        EventLog::append(
            &store,
            envelope(
                2,
                DomainEvent::RetrievalEventsRetired {
                    before_sequence: 2,
                    reason: "audit policy".to_string(),
                },
            ),
        )?;
        EventLog::append(&store, envelope(3, search_executed("live query")))?;
        EventLog::append(
            &store,
            envelope(
                4,
                DomainEvent::TickObserved {
                    at: LogicalTick::new(7),
                },
            ),
        )?;

        let scanned = EventLog::scan(&store, EventFilter { artifact_id: None })?;
        let kinds: Vec<&str> = scanned
            .iter()
            .map(|envelope| match envelope.event {
                DomainEvent::SearchExecuted { .. } => "search_executed",
                DomainEvent::RetrievalEventsRetired { .. } => "retrieval_events_retired",
                DomainEvent::TickObserved { .. } => "tick_observed",
                _ => "other",
            })
            .collect();
        // The retired search row stays on disk but is no longer decoded;
        // the marker, post-marker audit rows, and state-relevant rows replay.
        assert_eq!(
            kinds,
            [
                "retrieval_events_retired",
                "search_executed",
                "tick_observed"
            ]
        );
        assert_eq!(store.retrieval_retired_through()?, 2);
        Ok(())
    }
}
