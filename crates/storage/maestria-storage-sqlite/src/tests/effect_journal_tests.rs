use crate::SqliteStore;
use maestria_domain::{ScopeId, TaskId};
use maestria_ports::{
    EffectJournal, EffectJournalIntent, EffectJournalStatus, HarnessRunId, PortError,
};

#[test]
fn journal_lifecycle_success() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let run_id = HarnessRunId::new(1);
    let task_id = TaskId::new(2);
    let scope_id = ScopeId::new(3);

    // 1. Intent
    let intent = EffectJournalIntent {
        run_id,
        task_id: Some(task_id),
        capability: "http_get".to_string(),
        command: "url".to_string(),
        scope_id,
        requested_generation: None,
    };
    let entry = store.record_intent(intent.clone())?;
    assert_eq!(entry.run_id, run_id);
    assert_eq!(entry.generation, 1);
    assert_eq!(entry.status, EffectJournalStatus::Intent);

    // 2. Scan in flight shows Intent
    let in_flight = store.scan_in_flight()?;
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].status, EffectJournalStatus::Intent);

    // 3. Start
    store.record_started(run_id, 1)?;
    let in_flight_started = store.scan_in_flight()?;
    assert_eq!(in_flight_started.len(), 1);
    assert_eq!(in_flight_started[0].status, EffectJournalStatus::Started);

    // 4. Current check
    assert!(store.is_current(run_id, 1)?);
    assert!(!store.is_current(run_id, 2)?);

    // 5. Complete
    store.record_terminal(run_id, 1, EffectJournalStatus::Completed)?;

    // 6. Scan in flight is empty
    let in_flight_empty = store.scan_in_flight()?;
    assert!(in_flight_empty.is_empty());
    Ok(())
}

#[test]
fn journal_intent_supersedes_in_flight() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let run_id = HarnessRunId::new(10);
    let scope_id = ScopeId::new(1);

    let intent1 = EffectJournalIntent {
        run_id,
        task_id: None,
        capability: "test".to_string(),
        command: "cmd1".to_string(),
        scope_id,
        requested_generation: None,
    };
    let entry1 = store.record_intent(intent1)?;
    assert_eq!(entry1.generation, 1);

    // Record another intent without finishing the first
    let intent2 = EffectJournalIntent {
        run_id,
        task_id: None,
        capability: "test".to_string(),
        command: "cmd2".to_string(),
        scope_id,
        requested_generation: None,
    };
    let entry2 = store.record_intent(intent2)?;
    assert_eq!(entry2.generation, 2);

    let in_flight = store.scan_in_flight()?;
    // Only generation 2 should be in flight, gen 1 was superseded
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].generation, 2);
    Ok(())
}

#[test]
fn journal_started_requires_intent() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let run_id = HarnessRunId::new(99);

    let result = store.record_started(run_id, 1);
    assert!(matches!(result, Err(PortError::NotFound)));
    Ok(())
}

#[test]
fn journal_terminal_requires_in_flight() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let run_id = HarnessRunId::new(42);

    let result = store.record_terminal(run_id, 1, EffectJournalStatus::Completed);
    assert!(matches!(result, Err(PortError::NotFound)));
    Ok(())
}
