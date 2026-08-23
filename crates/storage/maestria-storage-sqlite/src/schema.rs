use maestria_ports::PortError;
use rusqlite::Connection;

use crate::schema_validation::{
    table_exists, validate_domain_events_schema, validate_event_order,
    validate_stored_event_payloads,
};
use crate::sqlite_store::to_port_error;

/// Current storage schema version supported by this adapter.
///
/// Version 15 adds the durable learned-sparse promotion records table.
/// Version 14 adds the rebuildable provider realm-read-grant projection.
/// Version 13 is migrated forward exactly once; newer or older layouts are
/// rejected rather than guessed.
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 16;

/// Captures the pre-migration state of the database.
struct SchemaState {
    version: Option<i64>,
}

/// Probes the database for its recorded schema version.
///
/// Only the `schema_version` table is inspected: a missing table (fresh
/// database, or one whose migration never committed) reports `None`. No
/// legacy table or column shapes are probed.
fn detect_schema_state(connection: &Connection) -> Result<SchemaState, PortError> {
    let version = if table_exists(connection, "schema_version")? {
        connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(to_port_error)?
    } else {
        None
    };
    Ok(SchemaState { version })
}

const REALM_READ_GRANTS_DDL: &str = r#"CREATE TABLE IF NOT EXISTS realm_read_grants (
         token_digest TEXT NOT NULL PRIMARY KEY,
         provider_realm TEXT NOT NULL,
         consumer_realm TEXT NOT NULL,
         access TEXT NOT NULL CHECK(access IN ('search_only', 'search_and_open_evidence')),
         max_sensitivity TEXT NOT NULL CHECK(max_sensitivity IN ('public', 'internal', 'confidential', 'restricted')),
         max_results INTEGER NOT NULL CHECK(max_results BETWEEN 1 AND 100),
         max_evidence_bytes INTEGER NOT NULL CHECK(max_evidence_bytes BETWEEN 1 AND 65536),
         state TEXT NOT NULL CHECK(state IN ('active', 'revoked'))
     );
     CREATE INDEX IF NOT EXISTS idx_realm_read_grants_consumer
         ON realm_read_grants(consumer_realm);
     CREATE UNIQUE INDEX IF NOT EXISTS idx_realm_read_grants_active_consumer
         ON realm_read_grants(consumer_realm)
         WHERE state = 'active';"#;

const LEARNED_SPARSE_PROMOTION_RECORDS_DDL: &str = r#"CREATE TABLE IF NOT EXISTS learned_sparse_promotion_records (
         evaluation_id TEXT NOT NULL PRIMARY KEY,
         corpus_id TEXT NOT NULL,
         evaluation_date TEXT NOT NULL,
         report_hash TEXT NOT NULL,
         record_json TEXT NOT NULL,
         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
     );
     CREATE INDEX IF NOT EXISTS idx_learned_sparse_promotion_records_order
         ON learned_sparse_promotion_records(created_at DESC);"#;

const HYBRID_PROMOTION_RECORDS_DDL: &str = r#"CREATE TABLE IF NOT EXISTS hybrid_promotion_records (
         evaluation_id TEXT NOT NULL PRIMARY KEY,
         corpus_id TEXT NOT NULL,
         evaluation_date TEXT NOT NULL,
         report_hash TEXT NOT NULL,
         record_json TEXT NOT NULL,
         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
     );
     CREATE INDEX IF NOT EXISTS idx_hybrid_promotion_records_order
         ON hybrid_promotion_records(created_at DESC);"#;

/// SQL that bootstraps every table for a fresh database (all `IF NOT EXISTS`).
///
/// Foreign-key enforcement is enabled and validated by [`migrate`] before the
/// migration transaction starts. SQLite ignores a `PRAGMA foreign_keys` write
/// made while a transaction is active.
static BASE_SCHEMA_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        r#"CREATE TABLE IF NOT EXISTS schema_version (
         version INTEGER NOT NULL PRIMARY KEY,
         applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
     );
     CREATE TABLE IF NOT EXISTS artifacts (
         id INTEGER NOT NULL PRIMARY KEY,
         title TEXT NOT NULL,
         content_hash TEXT,
         index_status TEXT NOT NULL DEFAULT 'unindexed',
         parse_status TEXT,
         security_json TEXT NOT NULL DEFAULT '{}'
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
         text TEXT NOT NULL,
         node_id INTEGER,
         source_span_json TEXT,
         representations_json TEXT
     );
     CREATE INDEX IF NOT EXISTS idx_chunks_artifact_order
         ON chunks(artifact_id, chunk_order, id);
     CREATE TABLE IF NOT EXISTS cards (
         id INTEGER NOT NULL PRIMARY KEY,
         artifact_id INTEGER NOT NULL,
         title TEXT NOT NULL,
         body TEXT NOT NULL,
         node_id INTEGER,
         source_span_json TEXT,
         security_json TEXT NOT NULL DEFAULT '{}'
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
         observed_at INTEGER NOT NULL,
         security_json TEXT NOT NULL DEFAULT '{}'
     );
     CREATE INDEX IF NOT EXISTS idx_evidence_artifact
         ON evidence(artifact_id, id);
     CREATE TABLE IF NOT EXISTS domain_events (
         id INTEGER NOT NULL PRIMARY KEY,
         event_kind TEXT NOT NULL,
         artifact_id INTEGER,
         payload_json TEXT NOT NULL,
         payload_version INTEGER NOT NULL DEFAULT 2
     );
     CREATE INDEX IF NOT EXISTS idx_domain_events_artifact_id
         ON domain_events(artifact_id, id);
{realm_read_grants}
     CREATE TABLE IF NOT EXISTS id_counters (
         namespace TEXT PRIMARY KEY,
         next_id INTEGER NOT NULL DEFAULT 1
     );
     CREATE TABLE IF NOT EXISTS approval_requests (
         id INTEGER NOT NULL PRIMARY KEY,
         task_id INTEGER,
         effect_kind TEXT NOT NULL,
         risk_level TEXT NOT NULL,
         capability TEXT NOT NULL DEFAULT '',
         scope_id INTEGER NOT NULL DEFAULT 0,
         tick INTEGER NOT NULL,
         status TEXT NOT NULL DEFAULT 'pending'
     );
     CREATE TABLE IF NOT EXISTS effect_journal (
         run_id INTEGER NOT NULL,
         generation INTEGER NOT NULL,
         task_id INTEGER,
         capability TEXT NOT NULL,
         command TEXT NOT NULL,
         scope_id INTEGER NOT NULL,
         requested_generation INTEGER,
         status TEXT NOT NULL,
         feedback_json TEXT,
         PRIMARY KEY (run_id, generation)
     );
     CREATE TABLE IF NOT EXISTS learned_sparse_shadow_observations (
         id INTEGER PRIMARY KEY AUTOINCREMENT,
         schema_version INTEGER NOT NULL,
         query_id INTEGER NOT NULL,
         query_class TEXT NOT NULL,
         corpus_snapshot INTEGER NOT NULL,
         index_generation INTEGER NOT NULL,
         elapsed_ms INTEGER NOT NULL,
         observation_json TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_learned_sparse_shadow_observations_order
         ON learned_sparse_shadow_observations(id);
{sparse_promotion}
     {hybrid_promotion}
     CREATE TABLE IF NOT EXISTS learned_sparse_projections (
         identity_json TEXT NOT NULL PRIMARY KEY,
         generation_id INTEGER NOT NULL,
         corpus_snapshot INTEGER NOT NULL,
         namespace_json TEXT NOT NULL,
         fingerprint_json TEXT NOT NULL,
         lifecycle TEXT NOT NULL
     );
     CREATE UNIQUE INDEX IF NOT EXISTS idx_learned_sparse_projection_generation
         ON learned_sparse_projections(generation_id);
     CREATE TABLE IF NOT EXISTS learned_sparse_projection_documents (
         identity_json TEXT NOT NULL,
         chunk_id INTEGER NOT NULL,
         content_hash TEXT NOT NULL,
         vector_json TEXT NOT NULL,
         tombstoned INTEGER NOT NULL DEFAULT 0,
         PRIMARY KEY (identity_json, chunk_id),
         FOREIGN KEY (identity_json) REFERENCES learned_sparse_projections(identity_json)
             ON DELETE CASCADE
     );
     CREATE INDEX IF NOT EXISTS idx_learned_sparse_projection_documents_lookup
         ON learned_sparse_projection_documents(identity_json, tombstoned, chunk_id);
     CREATE TABLE IF NOT EXISTS learned_sparse_projection_meta (
         identity_json TEXT NOT NULL PRIMARY KEY,
         version INTEGER NOT NULL,
         FOREIGN KEY (identity_json) REFERENCES learned_sparse_projections(identity_json)
             ON DELETE CASCADE
    );
     CREATE TABLE IF NOT EXISTS projection_meta (
         key TEXT PRIMARY KEY,
         value TEXT NOT NULL
     );"#,
        maestria_sqlite_support::DEFAULT_SECURITY_JSON,
        maestria_sqlite_support::DEFAULT_SECURITY_JSON,
        maestria_sqlite_support::DEFAULT_SECURITY_JSON,
        realm_read_grants = REALM_READ_GRANTS_DDL,
        sparse_promotion = LEARNED_SPARSE_PROMOTION_RECORDS_DDL,
        hybrid_promotion = HYBRID_PROMOTION_RECORDS_DDL,
    )
});

/// Seeds the per-namespace `id_counters` rows from durable identity truth
/// so that fresh or migrated databases never start at the wrong counter value.
///
/// Scans `domain_events` for the maximum `claim_id`, `memory_candidate_id`, and
/// event-backed `approval_id`, and scans `approval_requests` for persisted
/// approval requests. Each counter is seeded at `max_id + 1` (or 1 if no
/// matching rows exist). Existing counters are advanced but never regressed.
fn next_counter_value(max_id: Option<i64>, namespace: &str) -> Result<i64, PortError> {
    max_id.map_or(Ok(1), |value| {
        value
            .checked_add(1)
            .ok_or_else(|| PortError::internal("id counter exhausted", namespace))
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

    // Approval IDs have two durable sources of truth: request rows and
    // ApprovalRecorded events. Both must advance the namespace so a crash
    // between event append and repository reconciliation cannot reuse an ID.
    let max_approval: Option<i64> = connection
        .query_row(
            "SELECT MAX(id) FROM (
                 SELECT id FROM approval_requests
                 UNION ALL
                 SELECT CAST(json_extract(payload_json, '$.approval_id') AS INTEGER)
                 FROM domain_events
                 WHERE event_kind = 'approval_recorded'
             )",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
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
        .execute_batch(&BASE_SCHEMA_SQL)
        .map_err(to_port_error)
}
/// Enables SQLite foreign-key enforcement before any migration transaction
/// begins, then verifies that SQLite accepted the setting.
fn ensure_foreign_keys(connection: &Connection) -> Result<(), PortError> {
    connection
        .pragma_update(None, "foreign_keys", 1_i64)
        .map_err(to_port_error)?;
    let enabled = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map_err(to_port_error)?;
    if enabled != 1 {
        return Err(PortError::InternalContext {
            context: "enable sqlite foreign-key enforcement",
            source: format!("PRAGMA foreign_keys reported {enabled}"),
        });
    }
    Ok(())
}

fn migrate_v14_to_v15(connection: &Connection) -> Result<(), PortError> {
    connection
        .execute_batch(LEARNED_SPARSE_PROMOTION_RECORDS_DDL)
        .map_err(to_port_error)
}

/// Drops the `sequence` column from `domain_events` (rows satisfy
/// `id == sequence` by construction). The column is part of the primary-key
/// table's implicit `sequence` unique index and the artifact index, so the
/// table is rebuilt rather than altered in place.
fn migrate_v15_to_v16(connection: &Connection) -> Result<(), PortError> {
    connection
        .execute_batch(
            "CREATE TABLE domain_events_v16 (
                 id INTEGER NOT NULL PRIMARY KEY,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL DEFAULT 2
             );
             INSERT INTO domain_events_v16 (id, event_kind, artifact_id, payload_json, payload_version)
                 SELECT id, event_kind, artifact_id, payload_json, payload_version FROM domain_events;
             DROP TABLE domain_events;
             ALTER TABLE domain_events_v16 RENAME TO domain_events;
             CREATE INDEX idx_domain_events_artifact_id
                 ON domain_events(artifact_id, id);",
        )
        .map_err(to_port_error)
}

fn migrate_v13_to_v14(connection: &Connection) -> Result<(), PortError> {
    connection
        .execute_batch(REALM_READ_GRANTS_DDL)
        .map_err(to_port_error)
}

/// Brings a database to [`CURRENT_SCHEMA_VERSION`].
///
/// Fresh databases (no recorded version) are created from
/// [`BASE_SCHEMA_SQL`], seeded with id counters, validated, and stamped with
/// the current version. Databases already at the current version are
/// re-validated idempotently. Any other recorded version is rejected: legacy
/// databases have no migration path.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    ensure_foreign_keys(connection)?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    let state = detect_schema_state(&transaction)?;
    // Payload-level validation decodes every stored event as JSON, which is
    // O(event log) per store open. That cost is only justified when the
    // schema (and therefore the writers) changed: fresh databases have no
    // events yet, and migrations are exactly the moments where column/payload
    // drift can be introduced. Steady-state opens keep the cheap structural
    // validators below.
    let schema_changed = matches!(state.version, None | Some(13) | Some(14) | Some(15));
    if let Some(version) = state.version
        && version != 13
        && version != 14
        && version != 15
        && version != CURRENT_SCHEMA_VERSION
    {
        return Err(PortError::InternalContext {
            context: "unsupported sqlite schema version",
            source: format!("{version}; expected 13, 14, 15, or {CURRENT_SCHEMA_VERSION}"),
        });
    }

    create_base_schema(&transaction)?;
    if state.version == Some(13) {
        migrate_v13_to_v14(&transaction)?;
    }
    if state.version == Some(13) || state.version == Some(14) {
        migrate_v14_to_v15(&transaction)?;
    }
    if state.version == Some(13) || state.version == Some(14) || state.version == Some(15) {
        migrate_v15_to_v16(&transaction)?;
    }
    seed_id_counters(&transaction)?;

    validate_domain_events_schema(&transaction)?;
    validate_event_order(&transaction)?;
    if schema_changed {
        validate_stored_event_payloads(&transaction)?;
    }

    if state.version.is_none()
        || state.version == Some(13)
        || state.version == Some(14)
        || state.version == Some(15)
    {
        transaction
            .execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                [CURRENT_SCHEMA_VERSION],
            )
            .map_err(to_port_error)?;
    }
    transaction.commit().map_err(to_port_error)
}
