use maestria_domain::{DomainEvent, DomainEventEnvelope, SearchTraceId};
use maestria_ports::{EventFilter, EventLog, PortError};
use rusqlite::params;
use std::collections::BTreeMap;

use crate::{
    events::{StoredEvent, read_stored_event},
    sqlite_store::{i64_to_u64, map_append_error, optional_u64_to_i64, to_port_error, u64_to_i64},
};
fn decode_scanned_events(stored: Vec<StoredEvent>) -> Result<Vec<DomainEventEnvelope>, PortError> {
    let mut trace_remap = BTreeMap::<SearchTraceId, SearchTraceId>::new();
    for event in &stored {
        let Some(old_trace) = event.raw_search_trace()? else {
            continue;
        };
        let canonical = event.clone().into_domain()?;
        let DomainEvent::SearchKnowledgeCompleted { outcome, .. } = canonical.event else {
            continue;
        };
        if let Some(previous) = trace_remap.insert(old_trace, outcome.trace)
            && previous != outcome.trace
        {
            return Err(PortError::Conflict {
                message: format!(
                    "legacy search trace maps to conflicting canonical identities: {old_trace}"
                ),
            });
        }
    }

    stored
        .into_iter()
        .map(|event| {
            let mut envelope = event.into_domain()?;
            if let DomainEvent::SearchExecuted {
                pack_metadata: Some(metadata),
                ..
            } = &mut envelope.event
            {
                if let Some(trace) = metadata.search_trace {
                    metadata.search_trace = trace_remap.get(&trace).copied().or(Some(trace));
                }
                if let maestria_domain::EvidencePackReproducibilityRecord::Frozen(replay) =
                    &mut metadata.reproducibility
                    && let Some(replacement) = trace_remap.get(&replay.trace)
                {
                    replay.trace = *replacement;
                }
            }
            Ok(envelope)
        })
        .collect()
}

impl EventLog for crate::SqliteStore {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let record = StoredEvent::from_domain(&event)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        let (count, max_id, max_sequence, mismatched): (i64, Option<i64>, Option<i64>, i64) =
            transaction
                .query_row(
                    "SELECT COUNT(*), MAX(id), MAX(sequence),
                            COALESCE(SUM(CASE WHEN id != sequence OR id < 1 THEN 1 ELSE 0 END), 0)
                     FROM domain_events",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(to_port_error)?;
        let count = u64::try_from(count).map_err(|_| PortError::InternalContext {
            context: "validate stored event count",
            source: "stored event count is negative".to_string(),
        })?;
        if count > 0 {
            if mismatched != 0 {
                return Err(PortError::Conflict {
                    message: "stored event log has mismatched ids and sequences".to_string(),
                });
            }
            let max_id = max_id.ok_or_else(|| PortError::InternalContext {
                context: "validate stored event log maximum id",
                source: "stored event log has no maximum id".to_string(),
            })?;
            let max_sequence = max_sequence.ok_or_else(|| PortError::InternalContext {
                context: "validate stored event log maximum sequence",
                source: "stored event log has no maximum sequence".to_string(),
            })?;
            let max_id = i64_to_u64(max_id)?;
            let max_sequence = i64_to_u64(max_sequence)?;
            if max_id != count || max_sequence != count {
                return Err(PortError::Conflict {
                    message: "stored event log is not contiguous".to_string(),
                });
            }
        }
        let expected_sequence = count + 1;
        if record.id != expected_sequence || record.sequence != expected_sequence {
            return Err(PortError::Conflict {
                message: format!(
                    "expected event id/sequence {expected_sequence}, got id {}, sequence {}",
                    record.id, record.sequence
                ),
            });
        }
        transaction
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    u64_to_i64(record.id)?,
                    u64_to_i64(record.sequence)?,
                    record.kind,
                    optional_u64_to_i64(record.artifact_id)?,
                    record.payload_json,
                    record.payload_version,
                ],
            )
            .map_err(map_append_error)?;
        transaction.commit().map_err(to_port_error)
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let connection = self.lock()?;
        let mut stored = Vec::new();

        if let Some(artifact_id) = filter.artifact_id {
            let mut statement = connection
                .prepare(
                    "SELECT e.id, e.sequence, e.event_kind, e.artifact_id, e.payload_json,
                            e.payload_version
                     FROM domain_events e
                     WHERE e.artifact_id = ?1
                     ORDER BY e.sequence ASC",
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
                .prepare(
                    "SELECT e.id, e.sequence, e.event_kind, e.artifact_id, e.payload_json,
                            e.payload_version
                     FROM domain_events e
                     ORDER BY e.sequence ASC",
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
