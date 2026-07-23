use maestria_ports::PortError;
use rusqlite::Connection;

use super::{CURRENT_SCHEMA_VERSION, SchemaState};
use crate::schema::{
    approval_migration::migrate_approval_recorded_payloads,
    supervision_migration::ensure_artifact_v3_columns,
};
use crate::{schema_validation::*, to_port_error};

/// Migrates a v1 database to the current version: validates domain_events columns,
/// adds the `payload_version` column with `DEFAULT 1` if not already present,
/// ensures artifact v3 columns, and records the new schema version.
pub(super) fn migrate_from_v1(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }

    if state.had_payload_version_column {
        validate_columns(connection, &DOMAIN_EVENTS_V2_COLUMNS)?;
    } else {
        validate_columns(connection, &DOMAIN_EVENTS_V1_COLUMNS)?;
        connection
            .execute_batch(
                "ALTER TABLE domain_events ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 1;",
            )
            .map_err(to_port_error)?;
    }
    ensure_artifact_v3_columns(connection)?;
    migrate_approval_recorded_payloads(connection)?;

    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Migrates a v2 database to the current version: validates domain_events columns
/// (requiring `payload_version` to exist), ensures artifact v3 columns, and records
/// the new schema version.
pub(super) fn migrate_from_v2(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }
    if state.had_payload_version_column {
        validate_columns(connection, &DOMAIN_EVENTS_V2_COLUMNS)?;
    } else {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: missing payload_version column".to_string(),
        });
    }
    ensure_artifact_v3_columns(connection)?;
    migrate_approval_recorded_payloads(connection)?;

    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Runs the final post-migration validation suite and commits the transaction.
/// Validates a v3 database and records the current schema version.
/// Table creation and counter seeding are handled by the shared
/// `create_base_schema` / `seed_id_counters` calls in [`migrate`].
pub(super) fn migrate_from_v3(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }
    ensure_artifact_v3_columns(connection)?;
    migrate_approval_recorded_payloads(connection)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Migrates a v4 database to v5: adds the `approval_requests` table.
pub(super) fn migrate_from_v4(
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
            "CREATE TABLE IF NOT EXISTS approval_requests (
                id INTEGER NOT NULL PRIMARY KEY,
                task_id INTEGER NOT NULL,
                effect_kind TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                capability TEXT NOT NULL DEFAULT '',
                scope_id INTEGER NOT NULL DEFAULT 0,
                tick INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending'
            );",
        )
        .map_err(to_port_error)?;

    // Data migration: old ApprovalRecorded payloads lack approval_id.
    // Allocate real IDs and rewrite the payload JSON before v5 replay.
    migrate_approval_recorded_payloads(connection)?;

    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Migrates a database that has no `schema_version` table (fresh install, or pre-v1
/// legacy). If `domain_events` already exists it is migrated to v2; otherwise the v2
/// columns are validated. Artifact v3 columns are ensured, and the current schema
/// version is recorded.
pub(super) fn migrate_from_fresh(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if state.had_domain_events_table {
        if state.had_payload_version_column {
            return Err(PortError::Internal {
                message:
                    "malformed sqlite schema: schema_version table missing for domain_events v2 schema"
                        .to_string(),
            });
        }
        validate_columns(connection, &DOMAIN_EVENTS_V1_COLUMNS)?;
        connection
            .execute_batch(
                "ALTER TABLE domain_events ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 1;",
            )
            .map_err(to_port_error)?;
    } else {
        validate_columns(connection, &DOMAIN_EVENTS_V2_COLUMNS)?;
    }
    ensure_artifact_v3_columns(connection)?;
    migrate_approval_recorded_payloads(connection)?;
    connection
        .execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Runs the final post-migration validation suite and commits the transaction.
pub(super) fn finalize_migration(transaction: rusqlite::Transaction) -> Result<(), PortError> {
    validate_domain_events_schema(&transaction)?;
    validate_event_order(&transaction)?;
    validate_stored_event_payloads(&transaction)?;
    transaction.commit().map_err(to_port_error)
}
