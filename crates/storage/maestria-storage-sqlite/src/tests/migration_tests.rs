use crate::SqliteStore;
use crate::schema::CURRENT_SCHEMA_VERSION;
use crate::sqlite_store::to_port_error;
use maestria_ports::*;
use rusqlite::{Connection, params};

fn assert_foreign_keys_enforced(store: &SqliteStore) -> Result<(), Box<dyn std::error::Error>> {
    let connection = store.lock()?;
    let foreign_keys: i64 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    assert_eq!(foreign_keys, 1);

    let dangling = connection.execute(
        "INSERT INTO artifact_chunks (artifact_id, related_id) VALUES (?1, ?2)",
        params![999_i64, 1_i64],
    );
    assert!(
        dangling.is_err(),
        "foreign-key enforcement must reject dangling children"
    );

    connection.execute(
        "INSERT INTO artifacts (id, title) VALUES (?1, ?2)",
        params![1_i64, "parent"],
    )?;
    connection.execute(
        "INSERT INTO artifact_chunks (artifact_id, related_id) VALUES (?1, ?2)",
        params![1_i64, 2_i64],
    )?;
    connection.execute("DELETE FROM artifacts WHERE id = ?1", [1_i64])?;
    let remaining_children: i64 = connection.query_row(
        "SELECT COUNT(*) FROM artifact_chunks WHERE artifact_id = ?1",
        [1_i64],
        |row| row.get(0),
    )?;
    assert_eq!(
        remaining_children, 0,
        "deleting a parent must cascade to children"
    );
    Ok(())
}

#[test]
fn fresh_store_enforces_foreign_keys() -> Result<(), Box<dyn std::error::Error>> {
    let fresh = SqliteStore::in_memory()?;
    assert_foreign_keys_enforced(&fresh)?;
    Ok(())
}

#[test]
fn migrations_are_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("store.db");

    SqliteStore::open(&path)?;
    SqliteStore::open(&path)?;

    let connection = Connection::open(path)?;
    let version: i64 =
        connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    for table in [
        "chunks",
        "cards",
        "card_claims",
        "evidence",
        "realm_read_grants",
    ] {
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1, "{table} table should exist");
    }
    Ok(())
}

#[test]
fn migrate_rejects_unsupported_schema_versions() -> Result<(), Box<dyn std::error::Error>> {
    // Legacy databases have no migration path: any recorded version other
    // than the current one must be rejected without being touched.
    for seeded_version in [1_i64, 12_i64] {
        let directory = tempfile::tempdir()?;
        let path = directory
            .path()
            .join(format!("legacy-v{seeded_version}.db"));
        {
            let connection = Connection::open(&path)?;
            connection.execute_batch(&format!(
                "CREATE TABLE schema_version (
                     version INTEGER NOT NULL PRIMARY KEY,
                     applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 INSERT INTO schema_version (version) VALUES ({seeded_version});
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL,
                     payload_version INTEGER NOT NULL DEFAULT 2
                 );"
            ))?;
        }

        let error = match SqliteStore::open(&path) {
            Ok(_) => {
                return Err(std::io::Error::other(format!(
                    "schema version {seeded_version} must be rejected"
                ))
                .into());
            }
            Err(error) => error,
        };
        assert!(
            error.is_internal(),
            "unsupported schema version must surface as an internal error: {error}"
        );

        // The rejected database must remain untouched: still its old version.
        let connection = Connection::open(&path)?;
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })?;
        assert_eq!(
            version, seeded_version,
            "rejected legacy database must not be stamped with the current version"
        );
    }
    Ok(())
}

#[test]
fn migrate_v13_creates_realm_read_grant_projection() -> Result<(), PortError> {
    use crate::schema::migrate;
    let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE schema_version (
                 version INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_version (version) VALUES (13);",
        )
        .map_err(to_port_error)?;

    migrate(&mut connection)?;
    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .map_err(to_port_error)?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'realm_read_grants'",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    assert_eq!(count, 1);
    migrate(&mut connection)?;
    Ok(())
}

#[test]
fn seed_id_counters_rejects_malformed_approval_requests_schema() -> Result<(), PortError> {
    let connection = Connection::open_in_memory().map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL
             );
             CREATE TABLE id_counters (
                 namespace TEXT NOT NULL PRIMARY KEY,
                 next_id INTEGER NOT NULL
             );
             CREATE TABLE approval_requests (request_id INTEGER);",
        )
        .map_err(to_port_error)?;

    let error = match crate::schema::seed_id_counters(&connection) {
        Err(error) => error,
        Ok(()) => {
            return Err(PortError::InternalContext {
                context: "malformed approval_requests must abort counter seeding",
                source: "seed_id_counters unexpectedly succeeded".to_string(),
            });
        }
    };
    assert!(
        error.is_downstream(),
        "schema query failures must remain typed storage errors: {error}"
    );
    Ok(())
}
