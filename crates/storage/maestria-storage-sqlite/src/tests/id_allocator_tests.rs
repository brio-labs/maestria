use crate::SqliteStore;
use maestria_ports::IdAllocator;

#[test]
fn allocates_independent_namespaces_from_fresh_db() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let c1 = store.allocate_claim_id()?;
    let mc1 = store.allocate_memory_candidate_id()?;
    let c2 = store.allocate_claim_id()?;
    let mc2 = store.allocate_memory_candidate_id()?;

    assert_eq!(c1.value(), 1, "first claim id");
    assert_eq!(mc1.value(), 1, "first candidate id");
    assert_eq!(c2.value(), 2, "second claim id");
    assert_eq!(mc2.value(), 2, "second candidate id");
    Ok(())
}

#[test]
fn allocation_is_monotonic_within_namespace() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let mut prev = 0u64;
    for _ in 0..5 {
        let id = store.allocate_claim_id()?;
        assert!(id.value() > prev, "claim ids must be strictly monotonic");
        prev = id.value();
    }
    Ok(())
}

#[test]
fn allocation_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::TempDir::new()?;
    let db_path = dir.path().join("test.db");

    // First session: allocate three claims and two candidates.
    {
        let store = SqliteStore::open(&db_path)?;
        assert_eq!(store.allocate_claim_id()?.value(), 1);
        assert_eq!(store.allocate_claim_id()?.value(), 2);
        assert_eq!(store.allocate_memory_candidate_id()?.value(), 1);
        assert_eq!(store.allocate_claim_id()?.value(), 3);
        assert_eq!(store.allocate_memory_candidate_id()?.value(), 2);
    }

    // Second session (simulates restart): counters must continue.
    {
        let store = SqliteStore::open(&db_path)?;
        assert_eq!(store.allocate_claim_id()?.value(), 4);
        assert_eq!(store.allocate_memory_candidate_id()?.value(), 3);
    }
    Ok(())
}

#[test]
fn allocation_starts_at_max_event_plus_one_when_replaying() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::TempDir::new()?;
    let db_path = dir.path().join("test.db");

    // Create database and insert events with known claim_id=7 and
    // candidate_id=12 before the allocator is ever used.
    {
        let conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS domain_events (
                 id INTEGER NOT NULL PRIMARY KEY,
                 sequence INTEGER NOT NULL UNIQUE,
                 event_kind TEXT NOT NULL,
                 artifact_id INTEGER,
                 payload_json TEXT NOT NULL,
                 payload_version INTEGER NOT NULL DEFAULT 2
             );
             CREATE TABLE IF NOT EXISTS id_counters (
                 namespace TEXT PRIMARY KEY,
                 next_id INTEGER NOT NULL DEFAULT 1
             );
             INSERT INTO schema_version (version) VALUES (4);
             INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
             VALUES (1, 1, 'artifact_registered', 1, '{\"event_kind\":\"artifact_registered\",\"artifact_id\":1,\"title\":\"test\"}', 2);
             INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
             VALUES (2, 2, 'claim_created', 1, '{\"event_kind\":\"claim_created\",\"claim_id\":7,\"artifact_id\":1,\"text\":\"test\",\"evidence_ids\":[]}', 2);
             INSERT INTO domain_events (id, sequence, event_kind, artifact_id, payload_json, payload_version)
             VALUES (3, 3, 'memory_candidate_created', NULL, '{\"event_kind\":\"memory_candidate_created\",\"candidate_id\":12,\"claim_id\":7,\"evidence_ids\":[],\"confidence_milli\":500}', 2);",
        )
        ?;
    }
    // We must first delete the stale counters so seed_id_counters
    // derives from the event log.
    {
        let conn = rusqlite::Connection::open(&db_path)?;
        conn.execute("DELETE FROM id_counters", [])?;
    }

    // Now open with SqliteStore — seed_id_counters will run and derive
    // next_claim=8, next_candidate=13 from the events.
    {
        let store = SqliteStore::open(&db_path)?;
        assert_eq!(store.allocate_claim_id()?.value(), 8);
        assert_eq!(store.allocate_memory_candidate_id()?.value(), 13);
    }
    Ok(())
}

#[test]
fn allocation_after_invalid_proposal_does_not_skip_ids() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::TempDir::new()?;
    let db_path = dir.path().join("test.db");

    let claim1;
    let cand1;
    {
        let store = SqliteStore::open(&db_path)?;
        claim1 = store.allocate_claim_id()?;
        cand1 = store.allocate_memory_candidate_id()?;
    }

    // Simulate: the proposal was rejected by the domain. IDs are
    // already consumed — a subsequent proposal must receive fresh IDs
    // so that no two entities ever share an identity.
    {
        let store = SqliteStore::open(&db_path)?;
        let claim2 = store.allocate_claim_id()?;
        let cand2 = store.allocate_memory_candidate_id()?;
        assert_eq!(
            claim2.value(),
            claim1.value() + 1,
            "rejected proposal must not cause ID reuse"
        );
        assert_eq!(
            cand2.value(),
            cand1.value() + 1,
            "rejected proposal must not cause ID reuse"
        );
    }
    Ok(())
}
