use crate::SqliteStore;
use crate::schema::CURRENT_SCHEMA_VERSION;
use crate::schema::migrate_approval_recorded_payloads;
use crate::schema_validation::table_has_column;
use crate::sqlite_store::to_port_error;
use maestria_domain::*;
use maestria_ports::*;
use rusqlite::{Connection, params};

use super::artifact;

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
fn fresh_and_migrated_stores_enforce_foreign_keys() -> Result<(), Box<dyn std::error::Error>> {
    let fresh = SqliteStore::in_memory()?;
    assert_foreign_keys_enforced(&fresh)?;

    let directory = tempfile::tempdir()?;
    let path = directory.path().join("existing.db");
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
    let migrated = SqliteStore::open(&path)?;
    assert_foreign_keys_enforced(&migrated)?;
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
    for table in ["chunks", "cards", "card_claims", "evidence"] {
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
        Err(error) if error.is_internal()
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
        Err(error) if error.is_invalid_input()
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
                security: SecurityMetadata::default(),
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
        Err(error) if error.is_invalid_input()
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

    let res = migrate(&mut connection);
    assert!(res.is_err_and(|e| e.is_internal()));
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

    assert!(SqliteStore::open(&path).is_err_and(|error| error.is_internal()));
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
    let stored = ArtifactRepository::get(&store, ArtifactId::new(1))?
        .ok_or(maestria_ports::PortError::NotFound)?;
    assert_eq!(stored.index_status, IndexStatus::Pending);

    Ok(())
}

#[test]
fn legacy_v7_migration_adds_security_json_and_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("v7-legacy.db");

    // Seed a v7-style database
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE artifacts (
                 id INTEGER NOT NULL PRIMARY KEY,
                 title TEXT NOT NULL,
                 content_hash TEXT,
                 index_status TEXT NOT NULL DEFAULT 'unindexed',
                 parse_status TEXT
             );
             CREATE TABLE cards (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 title TEXT NOT NULL,
                 body TEXT NOT NULL,
                 node_id INTEGER,
                 source_span_json TEXT
             );
             CREATE TABLE evidence (
                 id INTEGER NOT NULL PRIMARY KEY,
                 artifact_id INTEGER NOT NULL,
                 claim_id INTEGER,
                 kind_json TEXT NOT NULL,
                 excerpt TEXT NOT NULL,
                 observed_at INTEGER NOT NULL
             );
             CREATE TABLE domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL DEFAULT 2
             );
             CREATE TABLE schema_version (
                 version INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_version (version) VALUES (7);
             ",
        )?;
    }

    let store = SqliteStore::open(&path)?;

    // Verify the migration added v8 columns
    {
        let connection = store.lock()?;
        assert!(table_has_column(&connection, "artifacts", "security_json")?);
        assert!(table_has_column(&connection, "cards", "security_json")?);
        assert!(table_has_column(&connection, "evidence", "security_json")?);

        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .map_err(to_port_error)?;
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    // Verify defaults map correctly
    {
        let connection = store.lock()?;
        connection.execute("INSERT INTO artifacts (id, title) VALUES (1, 'test')", [])?;
        let security_json: String = connection.query_row(
            "SELECT security_json FROM artifacts WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let sec: maestria_domain::SecurityMetadata = serde_json::from_str(&security_json)?;
        assert_eq!(sec, maestria_domain::SecurityMetadata::default());
    }

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

#[test]
fn legacy_approval_mapping_skips_colliding_request_and_replays_deterministically()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let connection = store.lock()?;
    connection.execute(
        "INSERT INTO approval_requests
         (id, task_id, effect_kind, risk_level, capability, scope_id, tick, status)
         VALUES (1, 999, 'task_activation', 'medium', 'task_activation', 1, 0, 'pending')",
        [],
    )?;
    connection.execute(
        "INSERT INTO domain_events
         (id, sequence, event_kind, artifact_id, payload_json, payload_version)
         VALUES (1, 1, 'approval_recorded', NULL, ?1, 1)",
        [r#"{"event_kind":"approval_recorded","task_id":42,"approved":true,"from_status":"draft","to_status":"active"}"#],
    )?;
    connection.execute(
        "INSERT INTO domain_events
         (id, sequence, event_kind, artifact_id, payload_json, payload_version)
         VALUES (2, 2, 'approval_recorded', NULL, ?1, 1)",
        [r#"{"event_kind":"approval_recorded","approved":false,"from_status":null,"to_status":null}"#],
    )?;
    migrate_approval_recorded_payloads(&connection)?;

    let mapped: i64 = connection.query_row(
        "SELECT approval_id FROM approval_event_mapping WHERE event_id = 1",
        [],
        |row| row.get(0),
    )?;
    let taskless_mapped: i64 = connection.query_row(
        "SELECT approval_id FROM approval_event_mapping WHERE event_id = 2",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(taskless_mapped, 3);
    let taskless_task: Option<i64> = connection.query_row(
        "SELECT task_id FROM approval_requests WHERE id = ?1",
        [taskless_mapped],
        |row| row.get(0),
    )?;
    assert_eq!(taskless_task, None);
    migrate_approval_recorded_payloads(&connection)?;
    assert_eq!(
        mapped, 2,
        "legacy event must not reuse unrelated approval id"
    );
    let projected_task: i64 = connection.query_row(
        "SELECT task_id FROM approval_requests WHERE id = ?1",
        [mapped],
        |row| row.get(0),
    )?;
    assert_eq!(projected_task, 42);
    drop(connection);

    let events = store.scan(EventFilter { artifact_id: None })?;
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events.first(),
        Some(DomainEventEnvelope {
            event: DomainEvent::ApprovalRecorded {
                approval_id,
                task_id,
                approved: true,
                ..
            },
            ..
        }) if approval_id.value() == 2
            && task_id.as_ref().is_some_and(|id| id.value() == 42)
    ));
    assert!(matches!(
        events.get(1),
        Some(DomainEventEnvelope {
            event: DomainEvent::ApprovalRecorded {
                approval_id,
                task_id: None,
                approved: false,
                ..
            },
            ..
        }) if approval_id.value() == 3
    ));
    Ok(())
}
fn seed_v9_evidence_database(
    path: &std::path::Path,
    evidence_kind: Option<&str>,
    event_payload: Option<&str>,
    artifact_hash: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "CREATE TABLE schema_version (
             version INTEGER NOT NULL PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         INSERT INTO schema_version (version) VALUES (9);
         CREATE TABLE artifacts (
             id INTEGER NOT NULL PRIMARY KEY,
             title TEXT NOT NULL,
             content_hash TEXT,
             index_status TEXT NOT NULL DEFAULT 'unindexed',
             parse_status TEXT
         );
         CREATE TABLE evidence (
             id INTEGER NOT NULL PRIMARY KEY,
             artifact_id INTEGER NOT NULL,
             claim_id INTEGER,
             kind_json TEXT NOT NULL,
             excerpt TEXT NOT NULL,
             observed_at INTEGER NOT NULL
         );
         CREATE TABLE domain_events (
             id INTEGER NOT NULL PRIMARY KEY,
             sequence INTEGER NOT NULL UNIQUE,
             event_kind TEXT NOT NULL,
             artifact_id INTEGER,
             payload_json TEXT NOT NULL,
             payload_version INTEGER NOT NULL DEFAULT 2
         );",
    )?;
    connection.execute(
        "INSERT INTO artifacts (id, title, content_hash) VALUES (1, 'artifact', ?1)",
        [artifact_hash],
    )?;
    if let Some(kind_json) = evidence_kind {
        connection.execute(
            "INSERT INTO evidence
                 (id, artifact_id, claim_id, kind_json, excerpt, observed_at)
             VALUES (1, 1, NULL, ?1, 'excerpt', 1)",
            [kind_json],
        )?;
    }
    if let Some(payload_json) = event_payload {
        if let Some(hash) = artifact_hash {
            let pending_payload = format!(
                r#"{{"event_kind":"pending_index","artifact_id":1,"content_hash":"{hash}"}}"#
            );
            connection.execute(
                "INSERT INTO domain_events
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'pending_index', 1, ?1, 2)",
                [pending_payload],
            )?;
            connection.execute(
                "INSERT INTO domain_events
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (2, 2, 'evidence_recorded', 1, ?1, 2)",
                [payload_json],
            )?;
        } else {
            connection.execute(
                "INSERT INTO domain_events
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'evidence_recorded', 1, ?1, 2)",
                [payload_json],
            )?;
        }
    }
    Ok(())
}

#[test]
fn v9_evidence_snapshots_migrate_rows_and_events_and_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("v9-evidence.db");
    let hash = format!("sha256:{}", "a".repeat(64));
    let legacy_file = format!(
        r#"{{"kind":"file_span","path":"notes.md","start":1,"end":2,"content_hash":"{hash}","snapshot":7}}"#
    );
    let legacy_web = format!(
        r#"{{"kind":"web_snapshot","url":"https://example.test","snapshot":8,"fetched_at":3,"content_hash":"{hash}"}}"#
    );
    let event_payload = format!(
        r#"{{"event_kind":"evidence_recorded","evidence_id":1,"artifact_id":1,"claim_id":null,"evidence_kind":{legacy_web},"excerpt":"web","observed_at":3}}"#
    );
    seed_v9_evidence_database(&path, Some(&legacy_file), Some(&event_payload), None)?;

    let store = SqliteStore::open(&path)?;
    {
        let connection = store.lock()?;
        let kind_json: String =
            connection.query_row("SELECT kind_json FROM evidence WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        let kind: serde_json::Value = serde_json::from_str(&kind_json)?;
        assert_eq!(kind["snapshot"]["blob_id"], 7);
        assert_eq!(kind["snapshot"]["content_hash"], hash);

        let (payload_json, payload_version): (String, i64) = connection.query_row(
            "SELECT payload_json, payload_version FROM domain_events WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let payload: serde_json::Value = serde_json::from_str(&payload_json)?;
        assert_eq!(payload_version, 2);
        assert_eq!(payload["evidence_kind"]["snapshot"]["blob_id"], 8);
        assert_eq!(payload["evidence_kind"]["snapshot"]["content_hash"], hash);
    }
    drop(store);

    let reopened = SqliteStore::open(&path)?;
    let evidence =
        EvidenceRepository::get(&reopened, EvidenceId::new(1))?.ok_or(PortError::NotFound)?;
    assert!(matches!(
        evidence.kind,
        EvidenceKind::FileSpan { snapshot, .. }
            if snapshot.blob_id().value() == 7 && snapshot.content_hash().as_str() == hash
    ));
    let events = reopened.scan(EventFilter { artifact_id: None })?;
    assert!(matches!(
        events.first(),
        Some(DomainEventEnvelope {
            event: DomainEvent::EvidenceRecorded {
                kind: EvidenceKind::WebSnapshot { snapshot, .. },
                ..
            },
            ..
        }) if snapshot.blob_id().value() == 8 && snapshot.content_hash().as_str() == hash
    ));
    Ok(())
}

#[test]
fn v9_pdf_snapshots_migrate_rows_and_events_and_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("v9-pdf-evidence.db");
    let hash = format!("sha256:{}", "b".repeat(64));
    let legacy_span = r#"{"kind":"pdf_span","blob":21,"page_start":2,"page_end":4}"#;
    let legacy_region =
        r#"{"kind":"pdf_region","blob":22,"page":3,"x":4,"y":5,"width":6,"height":7}"#;
    let event_payload = format!(
        r#"{{"event_kind":"evidence_recorded","evidence_id":1,"artifact_id":1,"claim_id":null,"evidence_kind":{legacy_region},"excerpt":"region","observed_at":3}}"#
    );
    seed_v9_evidence_database(&path, Some(legacy_span), Some(&event_payload), Some(&hash))?;

    let store = SqliteStore::open(&path)?;
    {
        let connection = store.lock()?;
        let kind_json: String =
            connection.query_row("SELECT kind_json FROM evidence WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        let kind: serde_json::Value = serde_json::from_str(&kind_json)?;
        assert_eq!(kind["snapshot"]["blob_id"], 21);
        assert_eq!(kind["snapshot"]["content_hash"], hash);

        let payload_json: String = connection.query_row(
            "SELECT payload_json FROM domain_events WHERE id = 2",
            [],
            |row| row.get(0),
        )?;
        let payload: serde_json::Value = serde_json::from_str(&payload_json)?;
        assert_eq!(payload["evidence_kind"]["snapshot"]["blob_id"], 22);
        assert_eq!(payload["evidence_kind"]["snapshot"]["content_hash"], hash);
    }
    drop(store);

    let reopened = SqliteStore::open(&path)?;
    let evidence =
        EvidenceRepository::get(&reopened, EvidenceId::new(1))?.ok_or(PortError::NotFound)?;
    assert!(matches!(
        evidence.kind,
        EvidenceKind::PdfSpan { snapshot, .. }
            if snapshot.blob_id().value() == 21 && snapshot.content_hash().as_str() == hash
    ));
    let events = reopened.scan(EventFilter { artifact_id: None })?;
    assert!(matches!(
        events.get(1),
        Some(DomainEventEnvelope {
            event: DomainEvent::EvidenceRecorded {
                kind: EvidenceKind::PdfRegion { snapshot, .. },
                ..
            },
            ..
        }) if snapshot.blob_id().value() == 22 && snapshot.content_hash().as_str() == hash
    ));
    Ok(())
}

#[test]
fn v9_pdf_event_snapshots_follow_historical_reindex_hashes()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("v9-pdf-reindex.db");
    let hash_a = format!("sha256:{}", "a".repeat(64));
    let hash_b = format!("sha256:{}", "b".repeat(64));
    seed_v9_evidence_database(&path, None, None, Some(&hash_b))?;

    let connection = Connection::open(&path)?;
    let pending_a =
        format!(r#"{{"event_kind":"pending_index","artifact_id":1,"content_hash":"{hash_a}"}}"#);
    let pending_b =
        format!(r#"{{"event_kind":"pending_index","artifact_id":1,"content_hash":"{hash_b}"}}"#);
    let evidence_a = r#"{"event_kind":"evidence_recorded","evidence_id":1,"artifact_id":1,"claim_id":null,"evidence_kind":{"kind":"pdf_span","blob":31,"page_start":1,"page_end":1},"excerpt":"A","observed_at":2}"#;
    let evidence_b = r#"{"event_kind":"evidence_recorded","evidence_id":2,"artifact_id":1,"claim_id":null,"evidence_kind":{"kind":"pdf_region","blob":32,"page":2,"x":1,"y":2,"width":3,"height":4},"excerpt":"B","observed_at":4}"#;
    connection.execute(
        "INSERT INTO domain_events
             (id, sequence, event_kind, artifact_id, payload_json, payload_version)
         VALUES (1, 1, 'pending_index', 1, ?1, 2)",
        [pending_a],
    )?;
    connection.execute(
        "INSERT INTO domain_events
             (id, sequence, event_kind, artifact_id, payload_json, payload_version)
         VALUES (2, 2, 'evidence_recorded', 1, ?1, 2)",
        [evidence_a],
    )?;
    connection.execute(
        "INSERT INTO domain_events
             (id, sequence, event_kind, artifact_id, payload_json, payload_version)
         VALUES (3, 3, 'pending_index', 1, ?1, 2)",
        [pending_b],
    )?;
    connection.execute(
        "INSERT INTO domain_events
             (id, sequence, event_kind, artifact_id, payload_json, payload_version)
         VALUES (4, 4, 'evidence_recorded', 1, ?1, 2)",
        [evidence_b],
    )?;
    drop(connection);

    let store = SqliteStore::open(&path)?;
    drop(store);
    let reopened = SqliteStore::open(&path)?;
    let events = reopened.scan(EventFilter { artifact_id: None })?;
    assert!(matches!(
        events.get(1),
        Some(DomainEventEnvelope {
            event: DomainEvent::EvidenceRecorded {
                kind: EvidenceKind::PdfSpan { snapshot, .. },
                ..
            },
            ..
        }) if snapshot.content_hash().as_str() == hash_a
    ));
    assert!(matches!(
        events.get(3),
        Some(DomainEventEnvelope {
            event: DomainEvent::EvidenceRecorded {
                kind: EvidenceKind::PdfRegion { snapshot, .. },
                ..
            },
            ..
        }) if snapshot.content_hash().as_str() == hash_b
    ));
    Ok(())
}

#[test]
fn v9_evidence_snapshot_migration_rejects_malformed_row_with_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed-v9-evidence-row.db");
    let malformed = r#"{"kind":"file_span","path":"notes.md","start":1,"end":1,"content_hash":"not-a-hash","snapshot":null}"#;
    seed_v9_evidence_database(&path, Some(malformed), None, None)?;

    let error = match SqliteStore::open(&path) {
        Ok(_) => {
            return Err("malformed evidence row unexpectedly migrated".into());
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("evidence row 1"));

    let connection = Connection::open(path)?;
    let version: i64 =
        connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?;
    assert_eq!(version, 9);
    let stored: String =
        connection.query_row("SELECT kind_json FROM evidence WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    assert_eq!(stored, malformed);
    Ok(())
}

#[test]
fn v9_evidence_snapshot_migration_rejects_malformed_event_with_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed-v9-evidence-event.db");
    let malformed_kind = r#"{"kind":"web_snapshot","url":"https://example.test","snapshot":8,"fetched_at":3,"content_hash":"not-a-hash"}"#;
    let event_payload = format!(
        r#"{{"event_kind":"evidence_recorded","evidence_id":1,"artifact_id":1,"claim_id":null,"evidence_kind":{malformed_kind},"excerpt":"web","observed_at":3}}"#
    );
    seed_v9_evidence_database(&path, None, Some(&event_payload), None)?;

    let error = match SqliteStore::open(&path) {
        Ok(_) => {
            return Err("malformed evidence event unexpectedly migrated".into());
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("domain event 1"));

    let connection = Connection::open(path)?;
    let version: i64 =
        connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?;
    assert_eq!(version, 9);
    let stored: String = connection.query_row(
        "SELECT payload_json FROM domain_events WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stored, event_payload);
    Ok(())
}

#[test]
fn v9_pdf_snapshot_migration_rejects_invalid_owner_hash_with_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("malformed-v9-pdf-owner.db");
    let malformed_owner = "not-a-hash";
    let legacy_span = r#"{"kind":"pdf_span","blob":21,"page_start":2,"page_end":4}"#;
    seed_v9_evidence_database(&path, Some(legacy_span), None, Some(malformed_owner))?;

    let error = match SqliteStore::open(&path) {
        Ok(_) => {
            return Err("invalid PDF owner hash unexpectedly migrated".into());
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("evidence row 1"));
    Ok(())
}
