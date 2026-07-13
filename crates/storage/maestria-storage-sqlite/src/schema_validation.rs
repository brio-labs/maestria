use maestria_ports::PortError;
use rusqlite::{Connection, OptionalExtension, params};

use crate::{i64_to_u64, to_port_error};

pub(crate) const DOMAIN_EVENTS_V1_COLUMNS: [&str; 5] = [
    "id",
    "sequence",
    "event_kind",
    "artifact_id",
    "payload_json",
];
pub(crate) const DOMAIN_EVENTS_V2_COLUMNS: [&str; 6] = [
    "id",
    "sequence",
    "event_kind",
    "artifact_id",
    "payload_json",
    "payload_version",
];

pub(crate) fn table_exists(connection: &Connection, table: &str) -> Result<bool, PortError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    Ok(exists.is_some())
}

pub(crate) fn table_has_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, PortError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let name: String = row.get(1).map_err(to_port_error)?;
        if name == column {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(crate) fn validate_columns(
    connection: &Connection,
    required: &[&str],
) -> Result<(), PortError> {
    for column in required {
        if !table_has_column(connection, "domain_events", column)? {
            return Err(PortError::Internal {
                message: format!("malformed domain_events table missing required column {column}"),
            });
        }
    }

    Ok(())
}

pub(crate) fn validate_domain_events_schema(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(domain_events)")
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        columns.push((
            row.get::<_, String>(1).map_err(to_port_error)?,
            row.get::<_, String>(2).map_err(to_port_error)?,
            row.get::<_, i64>(3).map_err(to_port_error)?,
            row.get::<_, i64>(5).map_err(to_port_error)?,
        ));
    }

    for (name, expected_type, require_not_null, require_nullable, require_primary_key) in [
        ("id", "INTEGER", true, false, true),
        ("sequence", "INTEGER", true, false, false),
        ("event_kind", "TEXT", true, false, false),
        ("artifact_id", "INTEGER", false, true, false),
        ("payload_json", "TEXT", true, false, false),
        ("payload_version", "INTEGER", true, false, false),
    ] {
        let Some((_, actual_type, not_null, primary_key)) =
            columns.iter().find(|(column, _, _, _)| column == name)
        else {
            return Err(PortError::Internal {
                message: format!("malformed domain_events table missing required column {name}"),
            });
        };
        if !actual_type.trim().eq_ignore_ascii_case(expected_type)
            || (require_not_null && *not_null == 0)
            || (require_nullable && *not_null != 0)
            || (require_primary_key && *primary_key == 0)
        {
            return Err(PortError::Internal {
                message: format!("malformed domain_events column {name}"),
            });
        }
    }

    let indexes = {
        let mut statement = connection
            .prepare("PRAGMA index_list(domain_events)")
            .map_err(to_port_error)?;
        let mut rows = statement.query([]).map_err(to_port_error)?;
        let mut indexes = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            indexes.push((
                row.get::<_, String>(1).map_err(to_port_error)?,
                row.get::<_, i64>(2).map_err(to_port_error)? != 0,
            ));
        }
        indexes
    };
    let mut has_unique_sequence = false;
    let mut has_artifact_sequence_index = false;
    for (index_name, unique) in indexes {
        let quoted_name = index_name.replace('"', "\"\"");
        let mut statement = connection
            .prepare(&format!("PRAGMA index_info(\"{quoted_name}\")"))
            .map_err(to_port_error)?;
        let mut rows = statement.query([]).map_err(to_port_error)?;
        let mut index_columns = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            index_columns.push(row.get::<_, String>(2).map_err(to_port_error)?);
        }
        if unique && index_columns == ["sequence"] {
            has_unique_sequence = true;
        }
        if index_name == "idx_domain_events_artifact_sequence"
            && index_columns == ["artifact_id", "sequence"]
        {
            has_artifact_sequence_index = true;
        }
    }
    if !has_unique_sequence {
        return Err(PortError::Internal {
            message: "malformed domain_events table sequence is not unique".to_string(),
        });
    }
    if !has_artifact_sequence_index {
        return Err(PortError::Internal {
            message: "malformed domain_events artifact index".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_event_order(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare("SELECT id, sequence FROM domain_events ORDER BY sequence ASC")
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut expected = 1_u64;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let id = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
        let sequence = i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?;
        if id != expected || sequence != expected {
            return Err(PortError::Internal {
                message: format!(
                    "domain event log is not contiguous at expected {expected}: id {id}, sequence {sequence}"
                ),
            });
        }
        expected = expected.checked_add(1).ok_or_else(|| PortError::Internal {
            message: "domain event sequence exhausted u64 range".to_string(),
        })?;
    }
    Ok(())
}

pub(crate) fn validate_stored_event_payloads(connection: &Connection) -> Result<(), PortError> {
    let mut statement = connection
        .prepare(
            "SELECT event_kind, artifact_id, payload_json, payload_version
             FROM domain_events
             ORDER BY sequence ASC",
        )
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let stored_kind: String = row.get(0).map_err(to_port_error)?;
        let stored_artifact_id: Option<i64> = row.get(1).map_err(to_port_error)?;
        let payload_json: String = row.get(2).map_err(to_port_error)?;
        let payload_version: i64 = row.get(3).map_err(to_port_error)?;
        let payload = match payload_version {
            1 => {
                let legacy: crate::payloads::LegacyStoredEventPayload =
                    serde_json::from_str(&payload_json).map_err(crate::json_error)?;
                legacy.into_v2()?
            }
            2 => serde_json::from_str(&payload_json).map_err(crate::json_error)?,
            version => {
                return Err(PortError::Internal {
                    message: format!("unsupported event payload version {version}"),
                });
            }
        };
        let payload_kind = payload.kind()?;
        if stored_kind != payload_kind {
            return Err(PortError::Internal {
                message: format!(
                    "event kind column {stored_kind} does not match payload kind {payload_kind}",
                ),
            });
        }
        let stored_artifact_id = stored_artifact_id.map(i64_to_u64).transpose()?;
        if stored_artifact_id != payload.filter_artifact_id() {
            return Err(PortError::Internal {
                message: "event artifact_id column does not match payload".to_string(),
            });
        }
    }
    Ok(())
}
