use maestria_ports::PortError;
use rusqlite::Connection;

use super::{CURRENT_SCHEMA_VERSION, SchemaState};
use crate::{
    schema_validation::{table_exists, table_has_column, validate_columns},
    to_port_error,
};

pub(super) fn ensure_provenance_v7_columns(connection: &Connection) -> Result<(), PortError> {
    if !table_has_column(connection, "chunks", "node_id")? {
        connection
            .execute("ALTER TABLE chunks ADD COLUMN node_id INTEGER", [])
            .map_err(to_port_error)?;
    }
    if !table_has_column(connection, "chunks", "source_span_json")? {
        connection
            .execute("ALTER TABLE chunks ADD COLUMN source_span_json TEXT", [])
            .map_err(to_port_error)?;
    }
    if !table_has_column(connection, "chunks", "representations_json")? {
        connection
            .execute(
                "ALTER TABLE chunks ADD COLUMN representations_json TEXT",
                [],
            )
            .map_err(to_port_error)?;
    }

    if !table_has_column(connection, "cards", "node_id")? {
        connection
            .execute("ALTER TABLE cards ADD COLUMN node_id INTEGER", [])
            .map_err(to_port_error)?;
    }
    if !table_has_column(connection, "cards", "source_span_json")? {
        connection
            .execute("ALTER TABLE cards ADD COLUMN source_span_json TEXT", [])
            .map_err(to_port_error)?;
    }
    if !table_has_column(connection, "cards", "title")? {
        connection
            .execute("ALTER TABLE cards ADD COLUMN title TEXT", [])
            .map_err(to_port_error)?;
    }
    if !table_has_column(connection, "cards", "body")? {
        connection
            .execute("ALTER TABLE cards ADD COLUMN body TEXT", [])
            .map_err(to_port_error)?;
    }

    if !table_has_column(connection, "artifacts", "parse_status")? {
        connection
            .execute("ALTER TABLE artifacts ADD COLUMN parse_status TEXT", [])
            .map_err(to_port_error)?;
    }

    Ok(())
}

pub(super) fn migrate_from_v6(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }

    ensure_provenance_v7_columns(connection)?;

    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

pub(super) fn validate_at_v7(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }
    if !state.had_payload_version_column {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: missing payload_version column".to_string(),
        });
    }
    validate_columns(
        connection,
        &["id", "sequence", "event_kind", "payload_json"],
    )?;
    for (table, columns) in [
        (
            "artifacts",
            [
                "id",
                "title",
                "content_hash",
                "index_status",
                "parse_status",
            ]
            .as_slice(),
        ),
        (
            "chunks",
            [
                "id",
                "artifact_id",
                "chunk_order",
                "text",
                "node_id",
                "source_span_json",
                "representations_json",
            ]
            .as_slice(),
        ),
        (
            "cards",
            [
                "id",
                "artifact_id",
                "title",
                "body",
                "node_id",
                "source_span_json",
            ]
            .as_slice(),
        ),
    ] {
        for column in columns {
            if !table_has_column(connection, table, column)? {
                return Err(PortError::Internal {
                    message: format!("malformed sqlite schema: {table} missing {column}"),
                });
            }
        }
    }
    for table in ["approval_requests", "effect_journal"] {
        if !table_exists(connection, table)? {
            return Err(PortError::Internal {
                message: format!("malformed sqlite schema: {table} missing"),
            });
        }
    }
    Ok(())
}
