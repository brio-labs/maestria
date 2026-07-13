use crate::SqliteStore;
use crate::schema::CURRENT_SCHEMA_VERSION;
use crate::schema_validation::table_has_column;
use crate::to_port_error;
use maestria_domain::*;
use maestria_ports::*;
use rusqlite::{Connection, params};

use super::artifact;

#[test]
fn migrations_are_idempotent() {
    let directory = tempfile::tempdir().expect("test setup");
    let path = directory.path().join("store.db");

    SqliteStore::open(&path).expect("test setup");
    SqliteStore::open(&path).expect("test setup");

    let connection = Connection::open(path).expect("test setup");
    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .expect("test setup");
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    for table in ["chunks", "cards", "card_claims", "evidence"] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(count, 1, "{table} table should exist");
    }
}

#[test]
fn migration_rejects_event_metadata_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("mismatched-metadata.db");
    {
        let connection = Connection::open(&path)?;
        let payload = r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"artifact"}"#;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             CREATE TABLE domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL
             );",
        )?;
        connection.execute("INSERT INTO schema_version (version) VALUES (?1);", [2])?;
        connection.execute(
            "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', NULL, ?1, 2)",
            [payload],
        )?;
    }

    assert!(matches!(
        SqliteStore::open(&path),
        Err(PortError::Internal { .. })
    ));
    Ok(())
}

#[test]
fn legacy_migration_rejects_lossy_existing_payloads() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lossy-legacy.db");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)
                 VALUES (1, 1, 'chunk_registered', 1, '{\"event_kind\":\"chunk_registered\",\"chunk_id\":1,\"artifact_id\":1,\"order\":0}');",
        )?;
    }

    assert!(matches!(
        SqliteStore::open(&path),
        Err(PortError::InvalidInput { .. })
    ));
    Ok(())
}

#[test]
fn legacy_event_rows_migrate_and_reject_lossy_payloads() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("legacy.db");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );",
        )?;
    }

    let store = SqliteStore::open(&path)?;
    {
        let connection = store.lock()?;
        connection.execute(
            "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', 1, ?1, 1)",
            params![r#"{"event_kind":"artifact_registered","artifact_id":1,"title":"legacy"}"#],
        )?;
    }
    assert_eq!(
        store.scan(EventFilter { artifact_id: None })?,
        vec![DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(1),
                title: "legacy".to_string(),
            },
        }]
    );

    {
        let connection = store.lock()?;
        connection.execute(
            "INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (2, 2, 'relation_created', NULL, ?1, 1)",
            params![r#"{"event_kind":"relation_created","relation_id":7}"#],
        )?;
    }
    assert!(matches!(
        store.scan(EventFilter { artifact_id: None }),
        Err(PortError::InvalidInput { .. })
    ));

    let connection = store.lock()?;
    let has_payload_version: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('domain_events') WHERE name = 'payload_version'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(has_payload_version, 1);
    Ok(())
}

#[test]
fn migration_rejects_non_nullable_artifact_column() -> Result<(), PortError> {
    use crate::schema::migrate;
    let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (2);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER NOT NULL,
                     payload_json TEXT NOT NULL,
                     payload_version INTEGER NOT NULL
                 );",
        )
        .map_err(to_port_error)?;

    assert!(matches!(
        migrate(&mut connection),
        Err(PortError::Internal { message }) if message.contains("artifact_id")
    ));
    Ok(())
}

#[test]
fn legacy_migration_rejects_noncontiguous_event_rows() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed-legacy.db");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO schema_version (version) VALUES (1);
                 CREATE TABLE domain_events (
                     id INTEGER NOT NULL PRIMARY KEY,
                     sequence INTEGER NOT NULL UNIQUE,
                     event_kind TEXT NOT NULL,
                     artifact_id INTEGER,
                     payload_json TEXT NOT NULL
                 );
                 INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json)
                 VALUES (9, 9, 'artifact_registered', 1, '{\"event_kind\":\"artifact_registered\",\"artifact_id\":1,\"title\":\"legacy\"}');",
        )?;
    }

    assert!(matches!(
        SqliteStore::open(&path),
        Err(PortError::Internal { .. })
    ));
    Ok(())
}

#[test]
fn legacy_v1_migration_adds_content_hash_and_index_status() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("v1-legacy.db");

    // Seed a v1-style database: artifacts table without content_hash/index_status,
    // domain_events without payload_version, and no schema_version table.
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE artifacts (
                 id INTEGER NOT NULL PRIMARY KEY,
                 title TEXT NOT NULL
             );
             CREATE TABLE domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL
             );",
        )?;
    }

    // Opening should migrate the v1 schema to v3, adding the missing columns.
    let store = SqliteStore::open(&path)?;

    // Verify the migration added both v3 columns.
    {
        let connection = store.lock()?;
        assert!(table_has_column(&connection, "artifacts", "content_hash")?);
        assert!(table_has_column(&connection, "artifacts", "index_status")?);

        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .map_err(to_port_error)?;
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    // Reopen and verify idempotence: columns still present and version unchanged.
    drop(store);
    let store = SqliteStore::open(&path)?;
    {
        let connection = store.lock()?;
        assert!(table_has_column(&connection, "artifacts", "content_hash")?);
        assert!(table_has_column(&connection, "artifacts", "index_status")?);

        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .map_err(to_port_error)?;
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    // Exercise the new columns through the repository API.
    let mut a = artifact(1);
    a.content_hash = Some("sha256:abc123def456".to_string());
    a.index_status = IndexStatus::Pending;
    ArtifactRepository::put(&store, a.clone())?;
    let stored = ArtifactRepository::get(&store, ArtifactId::new(1))?.expect("artifact must exist");
    assert_eq!(stored.content_hash, Some("sha256:abc123def456".to_string()));
    assert_eq!(stored.index_status, IndexStatus::Pending);

    Ok(())
}
