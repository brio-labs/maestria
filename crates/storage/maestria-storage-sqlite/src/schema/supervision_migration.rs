use maestria_ports::PortError;
use rusqlite::Connection;

use super::{CURRENT_SCHEMA_VERSION, SchemaState};
use crate::{schema_validation::table_has_column, to_port_error};

pub(super) fn ensure_artifact_v3_columns(connection: &Connection) -> Result<(), PortError> {
    if !table_has_column(connection, "artifacts", "content_hash")? {
        connection
            .execute("ALTER TABLE artifacts ADD COLUMN content_hash TEXT", [])
            .map_err(to_port_error)?;
    }
    if !table_has_column(connection, "artifacts", "index_status")? {
        connection
            .execute(
                "ALTER TABLE artifacts ADD COLUMN index_status TEXT NOT NULL DEFAULT 'unindexed'",
                [],
            )
            .map_err(to_port_error)?;
    }
    Ok(())
}

pub(super) fn migrate_from_v5(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS effect_journal (
                run_id INTEGER NOT NULL,
                generation INTEGER NOT NULL,
                task_id INTEGER,
                capability TEXT NOT NULL,
                command TEXT NOT NULL,
                scope_id INTEGER NOT NULL,
                requested_generation INTEGER,
                status TEXT NOT NULL,
                PRIMARY KEY (run_id, generation)
            );",
        )
        .map_err(to_port_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}
