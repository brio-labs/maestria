use maestria_ports::PortError;
use rusqlite::Connection;

use crate::to_port_error;

use crate::schema_validation::*;
/// Current storage schema version supported by this adapter.
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 5;

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
     );
     CREATE TABLE IF NOT EXISTS approval_requests (
         id INTEGER NOT NULL PRIMARY KEY,
         task_id INTEGER NOT NULL,
         effect_kind TEXT NOT NULL,
         risk_level TEXT NOT NULL,
         capability TEXT NOT NULL DEFAULT '',
         scope_id INTEGER NOT NULL DEFAULT 0,
         tick INTEGER NOT NULL,
         status TEXT NOT NULL DEFAULT 'pending'
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

    // Seed approval counter: query approval_requests if the table exists,
    // otherwise start at 1. Silently skip if table is absent (pre-migration).
    let max_approval: Option<i64> = connection
        .query_row("SELECT MAX(id) FROM approval_requests", [], |row| {
            row.get(0)
        })
        .ok();
    let next_approval = next_counter_value(max_approval, "approval")?;
    connection
        .execute(
            "INSERT INTO id_counters (namespace, next_id) VALUES ('approval', ?1)
             ON CONFLICT(namespace) DO UPDATE SET next_id = MAX(next_id, excluded.next_id)",
            params![next_approval],
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
/// Rewrite old ApprovalRecorded payloads that lack `approval_id`,
/// allocating real IDs from the `id_counters` table.
fn migrate_approval_recorded_payloads(connection: &Connection) -> Result<(), PortError> {
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
        // Allocate a new approval ID.
        let next: i64 = connection
            .query_row(
                "UPDATE id_counters SET next_id = next_id + 1
                 WHERE namespace = 'approval' RETURNING next_id - 1",
                [],
                |row| row.get(0),
            )
            .map_err(to_port_error)?;
        let new_id = next;

        // Extract task_id and approved from the old payload.
        let task_id: i64 = extract_json_field(payload, "task_id")?;
        let approved: bool = extract_json_bool(payload, "approved")?;
        let status = if approved { "approved" } else { "denied" };

        // Derive legacy from/to status from the approved flag.
        let (from_status, to_status) = if approved {
            ("draft", "active")
        } else {
            ("draft", "draft")
        };

        // Rewrite the event payload to include approval_id and status fields.
        let new_payload = migrate_approval_payload_json(payload, new_id, from_status, to_status)?;
        connection
            .execute(
                "UPDATE domain_events SET payload_json = ?1 WHERE id = ?2",
                params![new_payload, event_id],
            )
            .map_err(to_port_error)?;

        // Also insert a row in approval_requests so reconciliation
        // does not error on a missing record.
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

/// Rewrite a legacy ApprovalRecorded payload JSON to include approval_id,
/// from_status, and to_status fields.
fn migrate_approval_payload_json(
    payload: &str,
    new_id: i64,
    from_status: &str,
    to_status: &str,
) -> Result<String, PortError> {
    let marker = "\"approval_recorded\"";
    let pos = payload.find(marker).ok_or_else(|| PortError::Internal {
        message: "malformed approval_recorded legacy payload".to_string(),
    })?;
    let insert_at = pos + marker.len();
    let mut result = String::with_capacity(payload.len() + 80);
    result.push_str(&payload[..insert_at]);
    result.push_str(&format!(
        ",\"approval_id\":{new_id},\"from_status\":\"{from_status}\",\"to_status\":\"{to_status}\""
    ));
    result.push_str(&payload[insert_at..]);
    Ok(result)
}

/// Extract an i64 field value from a JSON object string.
fn extract_json_field(payload: &str, field: &str) -> Result<i64, PortError> {
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

/// Extract a bool field value from a JSON object string.
fn extract_json_bool(payload: &str, field: &str) -> Result<bool, PortError> {
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

/// Migrates a v4 database to v5: adds the `approval_requests` table.
fn migrate_from_v4(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
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

/// Validates that a database already at v5 conforms to the expected tables.
fn validate_at_v5(connection: &Connection, state: &SchemaState) -> Result<(), PortError> {
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
    if !table_exists(connection, "approval_requests")? {
        return Err(PortError::Internal {
            message: "malformed sqlite schema: approval_requests table missing".to_string(),
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
        Some(4) => migrate_from_v4(&transaction, &state)?,
        Some(5) => validate_at_v5(&transaction, &state)?,
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
