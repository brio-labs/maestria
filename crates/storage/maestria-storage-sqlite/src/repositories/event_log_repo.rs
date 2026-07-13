use maestria_domain::DomainEventEnvelope;
use maestria_ports::{EventFilter, EventLog, PortError};
use rusqlite::params;

use crate::{
    events::{StoredEvent, read_stored_event},
    i64_to_u64, map_append_error, optional_u64_to_i64, to_port_error, u64_to_i64,
};

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
        let count = u64::try_from(count).map_err(|_| PortError::Internal {
            message: "stored event count is negative".to_string(),
        })?;
        if count > 0 {
            if mismatched != 0 {
                return Err(PortError::Conflict {
                    message: "stored event log has mismatched ids and sequences".to_string(),
                });
            }
            let max_id = max_id.ok_or_else(|| PortError::Internal {
                message: "stored event log has no maximum id".to_string(),
            })?;
            let max_sequence = max_sequence.ok_or_else(|| PortError::Internal {
                message: "stored event log has no maximum sequence".to_string(),
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
        let mut events = Vec::new();

        if let Some(artifact_id) = filter.artifact_id {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json, payload_version
                     FROM domain_events
                     WHERE artifact_id = ?1
                     ORDER BY sequence ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement
                .query(params![u64_to_i64(artifact_id.value())?])
                .map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                events.push(read_stored_event(row)?.into_domain()?);
            }
        } else {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json, payload_version
                     FROM domain_events
                     ORDER BY sequence ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement.query([]).map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                events.push(read_stored_event(row)?.into_domain()?);
            }
        }

        Ok(events)
    }
}
