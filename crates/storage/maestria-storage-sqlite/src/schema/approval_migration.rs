use maestria_ports::PortError;
use rusqlite::{Connection, OptionalExtension, params};

use crate::sqlite_store::to_port_error;
pub(crate) fn ensure_nullable_approval_task_id(connection: &Connection) -> Result<(), PortError> {
    use rusqlite::OptionalExtension;

    let not_null: Option<i64> = connection
        .query_row(
            "SELECT [notnull] FROM pragma_table_info('approval_requests')
             WHERE name = 'task_id'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    if not_null != Some(1) {
        return Ok(());
    }
    connection
        .execute_batch(
            "ALTER TABLE approval_requests RENAME TO approval_requests_legacy;
             CREATE TABLE approval_requests (
                 id INTEGER NOT NULL PRIMARY KEY,
                 task_id INTEGER,
                 effect_kind TEXT NOT NULL,
                 risk_level TEXT NOT NULL,
                 capability TEXT NOT NULL DEFAULT '',
                 scope_id INTEGER NOT NULL DEFAULT 0,
                 tick INTEGER NOT NULL,
                 status TEXT NOT NULL DEFAULT 'pending'
             );
             INSERT INTO approval_requests
                 (id, task_id, effect_kind, risk_level, capability, scope_id, tick, status)
             SELECT id, task_id, effect_kind, risk_level, capability, scope_id, tick, status
             FROM approval_requests_legacy;
             DROP TABLE approval_requests_legacy;",
        )
        .map_err(to_port_error)
}

/// Validate and project old ApprovalRecorded payloads that lack `approval_id`.
///
/// Domain event rows are immutable. The missing identity is upcast during
/// replay from a durable event-to-approval mapping; this migration creates
/// that mapping and the approval-request projection needed by the runtime.
pub(crate) fn migrate_approval_recorded_payloads(connection: &Connection) -> Result<(), PortError> {
    initialize_approval_migration(connection)?;
    for (event_id, payload) in load_legacy_approval_events(connection)? {
        migrate_legacy_approval_event(connection, event_id, &payload)?;
    }
    Ok(())
}

fn initialize_approval_migration(connection: &Connection) -> Result<(), PortError> {
    connection
        .execute(
            "INSERT OR IGNORE INTO id_counters (namespace, next_id)
             VALUES ('approval', 1)",
            [],
        )
        .map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS approval_event_mapping (
                 event_id INTEGER NOT NULL PRIMARY KEY,
                 approval_id INTEGER NOT NULL UNIQUE
             );",
        )
        .map_err(to_port_error)
}

fn load_legacy_approval_events(connection: &Connection) -> Result<Vec<(i64, String)>, PortError> {
    let mut stmt = connection
        .prepare(
            "SELECT id, payload_json FROM domain_events
             WHERE event_kind = 'approval_recorded'
               AND json_extract(payload_json, '$.approval_id') IS NULL",
        )
        .map_err(to_port_error)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(to_port_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(to_port_error)?;
    Ok(rows)
}

fn migrate_legacy_approval_event(
    connection: &Connection,
    event_id: i64,
    payload: &str,
) -> Result<(), PortError> {
    let task_id = extract_json_optional_field(payload, "task_id")?;
    let approved = extract_json_bool(payload, "approved")?;
    let expected_status = if approved { "approved" } else { "denied" };
    let mapped_id = find_approval_mapping(connection, event_id)?;
    let approval_id = match mapped_id {
        Some(id) => id,
        None => allocate_approval_id(connection)?,
    };

    reconcile_approval_request(connection, approval_id, task_id, expected_status, event_id)?;
    if mapped_id.is_none() {
        insert_approval_mapping(connection, event_id, approval_id)?;
    }
    advance_approval_counter(connection, approval_id)
}

fn find_approval_mapping(connection: &Connection, event_id: i64) -> Result<Option<i64>, PortError> {
    connection
        .query_row(
            "SELECT approval_id FROM approval_event_mapping WHERE event_id = ?1",
            [event_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)
}

fn allocate_approval_id(connection: &Connection) -> Result<i64, PortError> {
    let mut candidate: i64 = connection
        .query_row(
            "SELECT next_id FROM id_counters WHERE namespace = 'approval'",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    loop {
        if !approval_id_is_occupied(connection, candidate)? {
            return Ok(candidate);
        }
        candidate = candidate
            .checked_add(1)
            .ok_or_else(|| PortError::InternalContext {
                context: "legacy approval identity exhausted",
                source: candidate.to_string(),
            })?;
    }
}

fn approval_id_is_occupied(connection: &Connection, candidate: i64) -> Result<bool, PortError> {
    let occupied_request: Option<i64> = connection
        .query_row(
            "SELECT id FROM approval_requests WHERE id = ?1",
            [candidate],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    let occupied_mapping: Option<i64> = connection
        .query_row(
            "SELECT approval_id FROM approval_event_mapping WHERE approval_id = ?1",
            [candidate],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    let occupied_event: Option<i64> = connection
        .query_row(
            "SELECT id FROM domain_events
             WHERE event_kind = 'approval_recorded'
               AND json_extract(payload_json, '$.approval_id') = ?1
             LIMIT 1",
            [candidate],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    Ok(occupied_request.is_some() || occupied_mapping.is_some() || occupied_event.is_some())
}

fn reconcile_approval_request(
    connection: &Connection,
    approval_id: i64,
    task_id: Option<i64>,
    expected_status: &str,
    event_id: i64,
) -> Result<(), PortError> {
    let existing: Option<(Option<i64>, String)> = connection
        .query_row(
            "SELECT task_id, status FROM approval_requests WHERE id = ?1",
            [approval_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(to_port_error)?;
    if let Some((existing_task, existing_status)) = existing {
        if existing_task != task_id || existing_status != expected_status {
            return Err(PortError::Conflict {
                message: format!(
                    "legacy approval event {event_id} maps to unrelated approval {approval_id}"
                ),
            });
        }
    } else {
        connection
            .execute(
                "INSERT INTO approval_requests
                 (id, task_id, effect_kind, risk_level, capability, scope_id, tick, status)
                 VALUES (?1, ?2, 'legacy_approval', 'medium', 'legacy', 1, 0, ?3)",
                params![approval_id, task_id, expected_status],
            )
            .map_err(to_port_error)?;
    }
    Ok(())
}

fn insert_approval_mapping(
    connection: &Connection,
    event_id: i64,
    approval_id: i64,
) -> Result<(), PortError> {
    connection
        .execute(
            "INSERT INTO approval_event_mapping (event_id, approval_id)
             VALUES (?1, ?2)",
            params![event_id, approval_id],
        )
        .map_err(to_port_error)?;
    Ok(())
}

fn advance_approval_counter(connection: &Connection, approval_id: i64) -> Result<(), PortError> {
    let next_id = approval_id
        .checked_add(1)
        .ok_or_else(|| PortError::InternalContext {
            context: "legacy approval identity exhausted",
            source: approval_id.to_string(),
        })?;
    connection
        .execute(
            "UPDATE id_counters SET next_id = MAX(next_id, ?1)
             WHERE namespace = 'approval'",
            [next_id],
        )
        .map_err(to_port_error)?;
    Ok(())
}

fn extract_json_optional_field(
    payload: &str,
    field: &'static str,
) -> Result<Option<i64>, PortError> {
    let value: serde_json::Value =
        serde_json::from_str(payload).map_err(|_| PortError::InternalContext {
            context: "invalid legacy approval payload",
            source: field.to_string(),
        })?;
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| PortError::InternalContext {
                context: "invalid field value in legacy payload",
                source: field.to_string(),
            }),
    }
}

pub(crate) fn extract_json_bool(payload: &str, field: &'static str) -> Result<bool, PortError> {
    let key = format!("\"{field}\":");
    let start = payload
        .find(&key)
        .ok_or_else(|| PortError::InternalContext {
            context: "missing field in legacy payload",
            source: field.to_string(),
        })?;
    let after_key = start + key.len();
    let rest = payload[after_key..].trim_start();
    if rest.starts_with("true") {
        Ok(true)
    } else if rest.starts_with("false") {
        Ok(false)
    } else {
        Err(PortError::InternalContext {
            context: "invalid bool in legacy payload",
            source: field.to_string(),
        })
    }
}
