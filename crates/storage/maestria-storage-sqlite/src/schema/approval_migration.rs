use maestria_ports::PortError;
use rusqlite::Connection;

use crate::to_port_error;

/// Rewrite old ApprovalRecorded payloads that lack `approval_id`,
/// allocating real IDs from the `id_counters` table.
pub(crate) fn migrate_approval_recorded_payloads(connection: &Connection) -> Result<(), PortError> {
    use rusqlite::params;

    // Seed the approval counter so we have a namespace row.
    connection
        .execute(
            "INSERT OR IGNORE INTO id_counters (namespace, next_id)
             VALUES ('approval', 1)",
            [],
        )
        .map_err(to_port_error)?;

    // Find approval_recorded events whose payload lacks "approval_id".
    let mut stmt = connection
        .prepare(
            "SELECT id, payload_json FROM domain_events
             WHERE event_kind = 'approval_recorded'
               AND payload_json NOT LIKE '%\"approval_id\"%'",
        )
        .map_err(to_port_error)?;

    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(to_port_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(to_port_error)?;

    for (event_id, payload) in &rows {
        let next: i64 = connection
            .query_row(
                "UPDATE id_counters SET next_id = next_id + 1
                 WHERE namespace = 'approval' RETURNING next_id - 1",
                [],
                |row| row.get(0),
            )
            .map_err(to_port_error)?;
        let new_id = next;

        let task_id: i64 = extract_json_field(payload, "task_id")?;
        let approved: bool = extract_json_bool(payload, "approved")?;
        let status = if approved { "approved" } else { "denied" };

        let new_payload = migrate_approval_payload_json(payload, new_id)?;
        connection
            .execute(
                "UPDATE domain_events SET payload_json = ?1 WHERE id = ?2",
                params![new_payload, event_id],
            )
            .map_err(to_port_error)?;

        connection
            .execute(
                "INSERT OR IGNORE INTO approval_requests
                 (id, task_id, effect_kind, risk_level, capability, scope_id, tick, status)
                 VALUES (?1, ?2, 'legacy_approval', 'medium', 'legacy', 1, 0, ?3)",
                params![new_id, task_id, status],
            )
            .map_err(to_port_error)?;
    }

    Ok(())
}

pub(crate) fn migrate_approval_payload_json(
    payload: &str,
    new_id: i64,
) -> Result<String, PortError> {
    let marker = "\"approval_recorded\"";
    let pos = payload.find(marker).ok_or_else(|| PortError::Internal {
        message: "malformed approval_recorded legacy payload".to_string(),
    })?;
    let insert_at = pos + marker.len();
    let mut result = String::with_capacity(payload.len() + 30);
    result.push_str(&payload[..insert_at]);
    result.push_str(&format!(",\"approval_id\":{new_id}"));
    result.push_str(&payload[insert_at..]);
    Ok(result)
}

pub(crate) fn extract_json_field(payload: &str, field: &str) -> Result<i64, PortError> {
    let key = format!("\"{field}\":");
    let start = payload.find(&key).ok_or_else(|| PortError::Internal {
        message: format!("missing field {field} in legacy payload"),
    })?;
    let after_key = start + key.len();
    let value_str = &payload[after_key..];
    let end = value_str
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(value_str.len());
    value_str[..end]
        .parse::<i64>()
        .map_err(|_| PortError::Internal {
            message: format!("invalid {field} value in legacy payload"),
        })
}

pub(crate) fn extract_json_bool(payload: &str, field: &str) -> Result<bool, PortError> {
    let key = format!("\"{field}\":");
    let start = payload.find(&key).ok_or_else(|| PortError::Internal {
        message: format!("missing field {field} in legacy payload"),
    })?;
    let after_key = start + key.len();
    let rest = payload[after_key..].trim_start();
    if rest.starts_with("true") {
        Ok(true)
    } else if rest.starts_with("false") {
        Ok(false)
    } else {
        Err(PortError::Internal {
            message: format!("invalid {field} bool in legacy payload"),
        })
    }
}
