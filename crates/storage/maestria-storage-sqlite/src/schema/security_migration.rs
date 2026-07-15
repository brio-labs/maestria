use maestria_ports::PortError;
use rusqlite::Connection;

use super::{
    CURRENT_SCHEMA_VERSION, SchemaState,
    provenance_migration::{ensure_provenance_v7_columns, validate_at_v7},
};
use crate::{
    schema_validation::{table_exists, table_has_column},
    to_port_error,
};

pub(super) fn ensure_security_v8_columns(connection: &Connection) -> Result<(), PortError> {
    if !table_has_column(connection, "artifacts", "security_json")? {
        connection.execute(
            r#"ALTER TABLE artifacts ADD COLUMN security_json TEXT NOT NULL DEFAULT '{"trust_zone":"Untrusted","authority":"External","integrity":"Unverified","sensitivity":"Internal","review_status":"Unreviewed","quarantined":false,"prompt_injection_risk":false,"poisoning_flags":[],"read_allowed":true,"write_allowed":false,"scope_id":null}'"#,
            [],
        ).map_err(to_port_error)?;
    }

    if !table_has_column(connection, "cards", "security_json")? {
        connection.execute(
            r#"ALTER TABLE cards ADD COLUMN security_json TEXT NOT NULL DEFAULT '{"trust_zone":"Untrusted","authority":"External","integrity":"Unverified","sensitivity":"Internal","review_status":"Unreviewed","quarantined":false,"prompt_injection_risk":false,"poisoning_flags":[],"read_allowed":true,"write_allowed":false,"scope_id":null}'"#,
            [],
        ).map_err(to_port_error)?;
    }

    if !table_has_column(connection, "evidence", "security_json")? {
        connection.execute(
            r#"ALTER TABLE evidence ADD COLUMN security_json TEXT NOT NULL DEFAULT '{"trust_zone":"Untrusted","authority":"External","integrity":"Unverified","sensitivity":"Internal","review_status":"Unreviewed","quarantined":false,"prompt_injection_risk":false,"poisoning_flags":[],"read_allowed":true,"write_allowed":false,"scope_id":null}'"#,
            [],
        ).map_err(to_port_error)?;
    }

    Ok(())
}

pub(super) fn migrate_from_v7(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }

    ensure_provenance_v7_columns(connection)?;
    ensure_security_v8_columns(connection)?;

    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

pub(super) fn validate_at_v8(
    connection: &Connection,
    state: &SchemaState,
) -> Result<(), PortError> {
    validate_at_v7(connection, state)?;
    if !table_exists(connection, "schema_version")?
        || !table_exists(connection, "artifacts")?
        || !table_exists(connection, "evidence")?
        || !table_exists(connection, "cards")?
        || !state.had_domain_events_table
    {
        return Err(PortError::Internal {
            message: "v8 validation failed: missing tables".to_string(),
        });
    }

    for column in [
        "id",
        "title",
        "content_hash",
        "index_status",
        "parse_status",
        "security_json",
    ] {
        if !table_has_column(connection, "artifacts", column)? {
            return Err(PortError::Internal {
                message: format!("v8 artifacts table missing required column {column}"),
            });
        }
    }
    for column in [
        "id",
        "artifact_id",
        "title",
        "body",
        "node_id",
        "source_span_json",
        "security_json",
    ] {
        if !table_has_column(connection, "cards", column)? {
            return Err(PortError::Internal {
                message: format!("v8 cards table missing required column {column}"),
            });
        }
    }
    for column in [
        "id",
        "artifact_id",
        "claim_id",
        "kind_json",
        "excerpt",
        "observed_at",
        "security_json",
    ] {
        if !table_has_column(connection, "evidence", column)? {
            return Err(PortError::Internal {
                message: format!("v8 evidence table missing required column {column}"),
            });
        }
    }

    Ok(())
}
