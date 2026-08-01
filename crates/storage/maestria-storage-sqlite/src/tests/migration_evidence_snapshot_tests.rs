use crate::SqliteStore;
use maestria_domain::*;
use maestria_ports::*;
use rusqlite::Connection;

/// Seed a schema-v9 database: `schema_version`, `artifacts`, `evidence`,
/// and `domain_events` tables, optionally populated with a legacy evidence
/// row and/or a legacy `evidence_recorded` payload.
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
