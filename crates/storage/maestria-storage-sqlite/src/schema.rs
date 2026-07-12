use maestria_ports::PortError;
use rusqlite::Connection;

use crate::to_port_error;

use crate::schema_validation::*;
/// Current storage schema version supported by this adapter.
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 3;

/// Ensures the `artifacts` table carries the v3 columns `content_hash` and `index_status`.
fn ensure_artifact_v3_columns(connection: &Connection) -> Result<(), PortError> {
    if !table_has_column(connection, "artifacts", "content_hash")? {
        connection
            .execute_batch(
                "ALTER TABLE artifacts ADD COLUMN content_hash TEXT;
                 ALTER TABLE artifacts ADD COLUMN index_status TEXT NOT NULL DEFAULT 'unindexed';",
            )
            .map_err(to_port_error)?;
    }
    Ok(())
}

pub(crate) fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    let transaction = connection.transaction().map_err(to_port_error)?;

    let had_schema_version_table = table_exists(&transaction, "schema_version")?;
    let had_domain_events_table = table_exists(&transaction, "domain_events")?;
    let had_payload_version_column = if had_domain_events_table {
        table_has_column(&transaction, "domain_events", "payload_version")?
    } else {
        false
    };
    let schema_version = if had_schema_version_table {
        let maybe_version: Option<i64> = transaction
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(to_port_error)?;
        match maybe_version {
            Some(v) => Some(v),
            None => {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema_version table is empty".to_string(),
                });
            }
        }
    } else {
        None
    };

    transaction
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS schema_version (
                 version INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS artifacts (
                 id INTEGER NOT NULL PRIMARY KEY,
                 title TEXT NOT NULL,
                 content_hash TEXT,
                 index_status TEXT NOT NULL DEFAULT 'unindexed'
             );
             CREATE TABLE IF NOT EXISTS artifact_chunks (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS artifact_cards (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS artifact_claims (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS artifact_evidences (
                 artifact_id INTEGER NOT NULL,
                 related_id INTEGER NOT NULL,
                 PRIMARY KEY (artifact_id, related_id),
                 FOREIGN KEY (artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS chunks (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 chunk_order INTEGER NOT NULL,
                 text TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_artifact_order
                 ON chunks(artifact_id, chunk_order, id);
             CREATE TABLE IF NOT EXISTS cards (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 title TEXT NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_cards_artifact
                 ON cards(artifact_id, id);
             CREATE TABLE IF NOT EXISTS card_claims (
                 card_id INTEGER NOT NULL,
                 claim_id INTEGER NOT NULL,
                 PRIMARY KEY (card_id, claim_id),
                 FOREIGN KEY (card_id) REFERENCES cards(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS evidence (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 claim_id INTEGER,
                 kind_json TEXT NOT NULL,
                 excerpt TEXT NOT NULL,
                 observed_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_evidence_artifact
                 ON evidence(artifact_id, id);
             CREATE TABLE IF NOT EXISTS domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL DEFAULT 2
             );
             CREATE INDEX IF NOT EXISTS idx_domain_events_artifact_sequence
                 ON domain_events(artifact_id, sequence);",
        )
        .map_err(to_port_error)?;

    match schema_version {
        Some(1) => {
            if !had_domain_events_table {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: domain_events table missing".to_string(),
                });
            }

            if had_payload_version_column {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            } else {
                validate_columns(&transaction, &DOMAIN_EVENTS_V1_COLUMNS)?;
                transaction
                    .execute_batch(
                        "ALTER TABLE domain_events ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 1;",
                    )
                    .map_err(to_port_error)?;
            }
            ensure_artifact_v3_columns(&transaction)?;

            transaction
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                    [CURRENT_SCHEMA_VERSION],
                )
                .map_err(to_port_error)?;
        }
        Some(2) => {
            if !had_domain_events_table {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: domain_events table missing".to_string(),
                });
            }
            if had_payload_version_column {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            } else {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: missing payload_version column".to_string(),
                });
            }
            ensure_artifact_v3_columns(&transaction)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                    [CURRENT_SCHEMA_VERSION],
                )
                .map_err(to_port_error)?;
        }
        Some(3) => {
            if !had_domain_events_table {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: domain_events table missing".to_string(),
                });
            }
            if had_payload_version_column {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            } else {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: missing payload_version column".to_string(),
                });
            }
            if !table_has_column(&transaction, "artifacts", "content_hash")? {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: artifacts table missing content_hash column"
                        .to_string(),
                });
            }
            if !table_has_column(&transaction, "artifacts", "index_status")? {
                return Err(PortError::Internal {
                    message: "malformed sqlite schema: artifacts table missing index_status column"
                        .to_string(),
                });
            }
        }
        Some(version) => {
            return Err(PortError::Internal {
                message: format!(
                    "unsupported sqlite schema version {version}; expected {CURRENT_SCHEMA_VERSION}"
                ),
            });
        }
        None => {
            if had_domain_events_table {
                if had_payload_version_column {
                    return Err(PortError::Internal {
                        message:
                            "malformed sqlite schema: schema_version table missing for domain_events v2 schema"
                                .to_string(),
                    });
                }
                validate_columns(&transaction, &DOMAIN_EVENTS_V1_COLUMNS)?;
                transaction
                    .execute_batch(
                        "ALTER TABLE domain_events ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 1;",
                    )
                    .map_err(to_port_error)?;
            } else {
                validate_columns(&transaction, &DOMAIN_EVENTS_V2_COLUMNS)?;
            }
            ensure_artifact_v3_columns(&transaction)?;

            transaction
                .execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    [CURRENT_SCHEMA_VERSION],
                )
                .map_err(to_port_error)?;
        }
    }

    validate_domain_events_schema(&transaction)?;
    validate_event_order(&transaction)?;
    validate_stored_event_payloads(&transaction)?;
    transaction.commit().map_err(to_port_error)
}
