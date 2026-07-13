use maestria_ports::PortError;
use rusqlite::Connection;

use crate::to_port_error;

use crate::schema_validation::*;
/// Current storage schema version supported by this adapter.
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 4;

/// Ensures the `artifacts` table carries the v3 columns `content_hash` and `index_status`.
fn ensure_artifact_v3_columns(connection: &Connection) -> Result<(), PortError> {
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

/// Captures the pre-migration state of the database.
struct SchemaState {
    version: Option<i64>,
    had_domain_events_table: bool,
    had_payload_version_column: bool,
}

/// Probes the database for its current schema state before applying any DDL.
fn detect_schema_state(connection: &Connection) -> Result<SchemaState, PortError> {
    let had_schema_version_table = table_exists(connection, "schema_version")?;
    let had_domain_events_table = table_exists(connection, "domain_events")?;
    let had_payload_version_column = if had_domain_events_table {
        table_has_column(connection, "domain_events", "payload_version")?
    } else {
        false
    };
    let version = if had_schema_version_table {
        let maybe_version: Option<i64> = connection
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
    Ok(SchemaState {
        version,
        had_domain_events_table,
        had_payload_version_column,
    })
}

/// SQL that bootstraps every table for a fresh database (all `IF NOT EXISTS`).
const BASE_SCHEMA_SQL: &str = "PRAGMA foreign_keys = ON;
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
         ON domain_events(artifact_id, sequence);
     CREATE TABLE IF NOT EXISTS id_counters (
         namespace TEXT PRIMARY KEY,
         next_id INTEGER NOT NULL DEFAULT 1
     );";

/// Seeds the per-namespace `id_counters` rows from existing domain events
/// so that fresh or migrated databases never start at the wrong counter value.
///
/// Scans `domain_events` for the maximum `claim_id` and `memory_candidate_id`
/// already persisted, then seeds each counter at `max_id + 1` (or 1 if no
/// matching events exist). Existing counters are advanced but never regressed.
fn next_counter_value(max_id: Option<i64>, namespace: &str) -> Result<i64, PortError> {
    max_id.map_or(Ok(1), |value| {
        value.checked_add(1).ok_or_else(|| PortError::Internal {
            message: format!("{namespace} id counter exhausted"),
        })
    })
}

pub(crate) fn seed_id_counters(connection: &Connection) -> Result<(), PortError> {
    use rusqlite::params;

    let max_claim: Option<i64> = connection
        .query_row(
            "SELECT MAX(CAST(json_extract(payload_json, '$.claim_id') AS INTEGER))
             FROM domain_events WHERE event_kind = 'claim_created'",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    let next_claim = next_counter_value(max_claim, "claim")?;
    connection
        .execute(
            "INSERT INTO id_counters (namespace, next_id) VALUES ('claim', ?1)
             ON CONFLICT(namespace) DO UPDATE SET next_id = MAX(next_id, excluded.next_id)",
            params![next_claim],
        )
        .map_err(to_port_error)?;

    let max_candidate: Option<i64> = connection
        .query_row(
            "SELECT MAX(CAST(json_extract(payload_json, '$.candidate_id') AS INTEGER))
             FROM domain_events WHERE event_kind = 'memory_candidate_created'",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    let next_candidate = next_counter_value(max_candidate, "memory_candidate")?;
    connection
        .execute(
            "INSERT INTO id_counters (namespace, next_id) VALUES ('memory_candidate', ?1)
             ON CONFLICT(namespace) DO UPDATE SET next_id = MAX(next_id, excluded.next_id)",
            params![next_candidate],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Creates every table and index using `CREATE TABLE IF NOT EXISTS` — safe to call
/// on both fresh and existing databases.
fn create_base_schema(connection: &Connection) -> Result<(), PortError> {
    connection
        .execute_batch(BASE_SCHEMA_SQL)
        .map_err(to_port_error)
}

/// Migrates a v1 database to the current version: validates domain_events columns,
/// adds the `payload_version` column with `DEFAULT 1` if not already present,
/// ensures artifact v3 columns, and records the new schema version.
fn migrate_from_v1(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
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
fn migrate_from_v2(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
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
fn migrate_from_fresh(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
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

    connection
        .execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Runs the final post-migration validation suite and commits the transaction.
/// Validates a v3 database and records the current schema version.
/// Table creation and counter seeding are handled by the shared
/// `create_base_schema` / `seed_id_counters` calls in [`migrate`].
fn migrate_from_v3(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
    if !state.had_domain_events_table {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: domain_events table missing".to_string(),
        });
    }
    ensure_artifact_v3_columns(connection)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [CURRENT_SCHEMA_VERSION],
        )
        .map_err(to_port_error)?;
    Ok(())
}

/// Validates that a database already at v4 conforms to the expected tables.
fn validate_at_v4(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
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
    if !table_has_column(connection, "artifacts", "content_hash")? {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: artifacts table missing content_hash column"
                .to_string(),
        });
    }
    if !table_has_column(connection, "artifacts", "index_status")? {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: artifacts table missing index_status column"
                .to_string(),
        });
    }
    Ok(())
}

fn finalize_migration(transaction: rusqlite::Transaction) -> Result<(), PortError> {
    validate_domain_events_schema(&transaction)?;
    validate_event_order(&transaction)?;
    validate_stored_event_payloads(&transaction)?;
    transaction.commit().map_err(to_port_error)
}

/// Applies any necessary schema migrations so the database matches
/// [`CURRENT_SCHEMA_VERSION`]. Idempotent — safe to call on a database that is
/// already at the current version.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    let transaction = connection.transaction().map_err(to_port_error)?;
    let state = detect_schema_state(&transaction)?;
    create_base_schema(&transaction)?;
    seed_id_counters(&transaction)?;

    match state.version {
        Some(1) => migrate_from_v1(&transaction, &state)?,
        Some(2) => migrate_from_v2(&transaction, &state)?,
        Some(3) => migrate_from_v3(&transaction, &state)?,
        Some(4) => validate_at_v4(&transaction, &state)?,
        Some(version) => {
            return Err(PortError::Internal {
                message: format!(
                    "unsupported sqlite schema version {version}; expected {CURRENT_SCHEMA_VERSION}"
                ),
            });
        }
        None => migrate_from_fresh(&transaction, &state)?,
    }

    finalize_migration(transaction)
}
