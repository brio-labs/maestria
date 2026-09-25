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
        "learned_sparse_promotion_records",
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
fn migrate_v14_creates_learned_sparse_promotion_records() -> Result<(), PortError> {
    use crate::schema::migrate;
    let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE schema_version (
                 version INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_version (version) VALUES (14);
             CREATE TABLE realm_read_grants (
                 token_digest TEXT NOT NULL PRIMARY KEY,
                 provider_realm TEXT NOT NULL,
                 consumer_realm TEXT NOT NULL,
                 access TEXT NOT NULL,
                 max_sensitivity TEXT NOT NULL,
                 max_results INTEGER NOT NULL,
                 max_evidence_bytes INTEGER NOT NULL,
                 state TEXT NOT NULL
             );",
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
             WHERE type = 'table' AND name = 'learned_sparse_promotion_records'",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    assert_eq!(count, 1);
    migrate(&mut connection)?;
    Ok(())
}

#[test]
fn migrate_v15_drops_domain_events_sequence_column() -> Result<(), PortError> {
    use crate::schema::migrate;
    let mut connection = Connection::open_in_memory().map_err(to_port_error)?;
    connection
        .execute_batch(
            "CREATE TABLE schema_version (
                 version INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             INSERT INTO schema_version (version) VALUES (15);
             CREATE TABLE domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL DEFAULT 2
             );
             CREATE INDEX idx_domain_events_artifact_sequence
                 ON domain_events(artifact_id, sequence);
             CREATE TABLE id_counters (
                 namespace TEXT PRIMARY KEY,
                 next_id INTEGER NOT NULL DEFAULT 1
             );
             INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
                 VALUES (1, 1, 'artifact_registered', 1, '{\"event_kind\":\"artifact_registered\",\"artifact_id\":1,\"title\":\"legacy\",\"security\":{\"trust_zone\":\"untrusted\",\"authority\":\"external\",\"integrity\":\"unverified\",\"sensitivity\":\"internal\",\"review_status\":\"unreviewed\",\"prompt_injection_risk\":false,\"poisoning_flags\":[],\"read_allowed\":true,\"write_allowed\":false,\"scope_id\":null}}', 6);",
        )
        .map_err(to_port_error)?;

    migrate(&mut connection)?;
    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .map_err(to_port_error)?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    // The legacy row survives with its identity; the sequence column is gone.
    let (id, kind): (i64, String) = connection
        .query_row(
            "SELECT id, event_kind FROM domain_events WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(to_port_error)?;
    assert_eq!(id, 1);
    assert_eq!(kind, "artifact_registered");
    let has_sequence: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('domain_events') WHERE name = 'sequence'",
            [],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;
    assert_eq!(has_sequence, 0);
    Ok(())
}

struct LegacyGrantRows<'a> {
    provider_realm: &'a str,
    consumer_realm: &'a str,
    revoked_consumer_realm: &'a str,
    token_digest: &'a str,
    revoked_token_digest: &'a str,
    legacy_payload: &'a str,
    revoked_issue_payload: &'a str,
    revoke_payload: &'a str,
}

fn seed_migrate_and_check_legacy_history(
    path: &std::path::Path,
    rows: LegacyGrantRows<'_>,
) -> Result<Connection, Box<dyn std::error::Error>> {
    let LegacyGrantRows {
        provider_realm,
        consumer_realm,
        revoked_consumer_realm,
        token_digest,
        revoked_token_digest,
        legacy_payload,
        revoked_issue_payload,
        revoke_payload,
    } = rows;
    let mut connection = Connection::open(path)?;
    connection.execute_batch(
        "CREATE TABLE schema_version (
             version INTEGER NOT NULL PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         INSERT INTO schema_version (version) VALUES (16);
         CREATE TABLE domain_events (
             id INTEGER NOT NULL PRIMARY KEY,
             event_kind TEXT NOT NULL,
             artifact_id INTEGER,
             payload_json TEXT NOT NULL,
             payload_version INTEGER NOT NULL DEFAULT 2
         );
         CREATE INDEX idx_domain_events_artifact_id
             ON domain_events(artifact_id, id);
         CREATE TABLE realm_read_grants (
             token_digest TEXT NOT NULL PRIMARY KEY,
             provider_realm TEXT NOT NULL,
             consumer_realm TEXT NOT NULL,
             access TEXT NOT NULL,
             max_sensitivity TEXT NOT NULL,
             max_results INTEGER NOT NULL,
             max_evidence_bytes INTEGER NOT NULL,
             state TEXT NOT NULL
         );",
    )?;
    for (id, kind, payload) in [
        (1_i64, "realm_read_grant_issued", legacy_payload),
        (2_i64, "realm_read_grant_issued", revoked_issue_payload),
        (3_i64, "realm_read_grant_revoked", revoke_payload),
    ] {
        connection.execute(
            "INSERT INTO domain_events (id, event_kind, artifact_id, payload_json, payload_version)
             VALUES (?1, ?2, NULL, ?3, 1)",
            params![id, kind, payload],
        )?;
    }
    for (digest, consumer, state) in [
        (token_digest, consumer_realm, "active"),
        (revoked_token_digest, revoked_consumer_realm, "revoked"),
    ] {
        connection.execute(
            "INSERT INTO realm_read_grants
                 (token_digest, provider_realm, consumer_realm, access, max_sensitivity,
                  max_results, max_evidence_bytes, state)
             VALUES (?1, ?2, ?3, 'search_only', 'public', 2, 128, ?4)",
            params![digest, provider_realm, consumer, state],
        )?;
    }
    crate::schema::migrate(&mut connection)?;
    let history = connection
        .prepare("SELECT id, payload_json FROM domain_events ORDER BY id")?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        history,
        [
            (1, legacy_payload.to_owned()),
            (2, revoked_issue_payload.to_owned()),
            (3, revoke_payload.to_owned()),
        ]
    );
    Ok(connection)
}

fn assert_legacy_grant_replay(
    path: &std::path::Path,
    active_digest: &str,
    revoked_digest: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..2 {
        let store = SqliteStore::open(path)?;
        let events = EventLog::scan(&store, EventFilter { artifact_id: None })?;
        let replayed = maestria_domain::replay_events(events)?;
        assert_eq!(replayed.realm_read_grants.len(), 2);
        let active = replayed
            .realm_read_grants
            .values()
            .find(|grant| grant.token_digest().as_str() == active_digest)
            .ok_or("active legacy grant did not replay")?;
        assert_eq!(active.expires_at().unix_seconds(), 1);
        assert!(active.allowed_roots().is_none());
        let revoked = replayed
            .realm_read_grants
            .values()
            .find(|grant| grant.token_digest().as_str() == revoked_digest)
            .ok_or("revoked legacy grant did not replay")?;
        assert_eq!(revoked.expires_at().unix_seconds(), 1);
        assert_eq!(
            revoked.state(),
            maestria_domain::RealmReadGrantState::Revoked
        );
        assert!(revoked.allowed_roots().is_none());
    }
    Ok(())
}

#[test]
fn v16_grants_expire_deterministically_without_rewriting_event_history()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("legacy-grants.db");
    let token_digest = "a".repeat(64);
    let provider_realm = "b".repeat(64);
    let consumer_realm = "c".repeat(64);
    let revoked_token_digest = "d".repeat(64);
    let revoked_consumer_realm = "e".repeat(64);
    #[derive(serde::Serialize)]
    struct LegacyGrantIssue<'a> {
        event_kind: &'static str,
        token_digest: &'a str,
        provider_realm: &'a str,
        consumer_realm: &'a str,
        access: &'static str,
        max_sensitivity: &'static str,
        max_results: usize,
        max_evidence_bytes: usize,
    }
    #[derive(serde::Serialize)]
    struct LegacyGrantRevocation<'a> {
        event_kind: &'static str,
        token_digest: &'a str,
    }
    let issue = LegacyGrantIssue {
        event_kind: "realm_read_grant_issued",
        token_digest: &token_digest,
        provider_realm: &provider_realm,
        consumer_realm: &consumer_realm,
        access: "search_only",
        max_sensitivity: "public",
        max_results: 2,
        max_evidence_bytes: 128,
    };
    let legacy_payload = serde_json::to_string(&issue)?;
    let revoked_issue_payload = serde_json::to_string(&LegacyGrantIssue {
        token_digest: &revoked_token_digest,
        consumer_realm: &revoked_consumer_realm,
        ..issue
    })?;
    let revoke_payload = serde_json::to_string(&LegacyGrantRevocation {
        event_kind: "realm_read_grant_revoked",
        token_digest: &revoked_token_digest,
    })?;
    {
        let connection = seed_migrate_and_check_legacy_history(
            &path,
            LegacyGrantRows {
                provider_realm: &provider_realm,
                consumer_realm: &consumer_realm,
                revoked_consumer_realm: &revoked_consumer_realm,
                token_digest: &token_digest,
                revoked_token_digest: &revoked_token_digest,
                legacy_payload: &legacy_payload,
                revoked_issue_payload: &revoked_issue_payload,
                revoke_payload: &revoke_payload,
            },
        )?;
        let projection = connection
            .prepare(
                "SELECT token_digest, expires_at_unix_seconds, state, allowed_roots_json
                 FROM realm_read_grants ORDER BY token_digest",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            projection,
            [
                (token_digest.clone(), 1, "active".to_string(), None),
                (revoked_token_digest.clone(), 1, "revoked".to_string(), None,),
            ]
        );
    }

    assert_legacy_grant_replay(&path, &token_digest, &revoked_token_digest)?;
    Ok(())
}

#[test]
fn v17_grants_gain_nullable_roots_without_changing_legacy_state()
-> Result<(), Box<dyn std::error::Error>> {
    let mut connection = Connection::open_in_memory()?;
    connection.execute_batch(
        "CREATE TABLE schema_version (
             version INTEGER NOT NULL PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         INSERT INTO schema_version (version) VALUES (17);
         CREATE TABLE domain_events (
             id INTEGER NOT NULL PRIMARY KEY,
             event_kind TEXT NOT NULL,
             artifact_id INTEGER,
             payload_json TEXT NOT NULL,
             payload_version INTEGER NOT NULL DEFAULT 2
         );
         CREATE TABLE realm_read_grants (
             token_digest TEXT NOT NULL PRIMARY KEY,
             provider_realm TEXT NOT NULL,
             consumer_realm TEXT NOT NULL,
             access TEXT NOT NULL,
             max_sensitivity TEXT NOT NULL,
             max_results INTEGER NOT NULL,
             max_evidence_bytes INTEGER NOT NULL,
             expires_at_unix_seconds INTEGER NOT NULL,
             state TEXT NOT NULL
         );",
    )?;
    let active = (
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        4_321_i64,
        "active",
    );
    let revoked = (
        "d".repeat(64),
        "e".repeat(64),
        "f".repeat(64),
        5_432_i64,
        "revoked",
    );
    for (digest, provider, consumer, expiry, state) in [&active, &revoked] {
        connection.execute(
            "INSERT INTO realm_read_grants
                 (token_digest, provider_realm, consumer_realm, access, max_sensitivity,
                  max_results, max_evidence_bytes, expires_at_unix_seconds, state)
             VALUES (?1, ?2, ?3, 'search_only', 'public', 2, 128, ?4, ?5)",
            params![digest, provider, consumer, expiry, state],
        )?;
    }

    crate::schema::migrate(&mut connection)?;

    let version: i64 =
        connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    let rows = connection
        .prepare(
            "SELECT token_digest, provider_realm, consumer_realm, access, max_sensitivity,
                    max_results, max_evidence_bytes, expires_at_unix_seconds, state,
                    allowed_roots_json
             FROM realm_read_grants ORDER BY token_digest",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        rows,
        [
            (
                active.0.clone(),
                active.1.clone(),
                active.2.clone(),
                "search_only".to_string(),
                "public".to_string(),
                2,
                128,
                active.3,
                active.4.to_string(),
                None::<String>,
            ),
            (
                revoked.0.clone(),
                revoked.1.clone(),
                revoked.2.clone(),
                "search_only".to_string(),
                "public".to_string(),
                2,
                128,
                revoked.3,
                revoked.4.to_string(),
                None::<String>,
            ),
        ]
    );
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
