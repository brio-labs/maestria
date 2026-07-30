use std::num::NonZeroUsize;

use maestria_domain::{CorpusSnapshotId, IndexGenerationId, QueryId};
use maestria_ports::{
    EventLog, LEARNED_SPARSE_SHADOW_SCHEMA_VERSION, LearnedSparseObservationRepository,
    LearnedSparseQueryClass, LearnedSparseShadowObservation, LearnedSparseShadowRoute,
};
use tempfile::tempdir;

use crate::SqliteStore;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn limit(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "test limit must be positive".into())
}

fn observation(query_id: u64) -> LearnedSparseShadowObservation {
    LearnedSparseShadowObservation {
        schema_version: LEARNED_SPARSE_SHADOW_SCHEMA_VERSION,
        query_id: QueryId::new(query_id),
        query_class: LearnedSparseQueryClass::ExactLiteral,
        route: LearnedSparseShadowRoute::Shadow,
        corpus_snapshot: CorpusSnapshotId::new(7),
        index_generation: IndexGenerationId::new(11),
        elapsed_ms: 3,
        lanes: Vec::new(),
    }
}

#[test]
fn shadow_observations_survive_restart_and_retention_pruning() -> TestResult {
    let directory = tempdir()?;
    let path = directory.path().join("instance.sqlite");
    let first = SqliteStore::open(&path)?;
    LearnedSparseObservationRepository::append_observation(&first, observation(1))?;
    LearnedSparseObservationRepository::append_observation(&first, observation(2))?;
    LearnedSparseObservationRepository::prune_observations(&first, limit(1)?)?;

    let reopened = SqliteStore::open(&path)?;
    let stored = LearnedSparseObservationRepository::scan_observations(&reopened, limit(8)?)?;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].query_id, QueryId::new(2));
    Ok(())
}

#[test]
fn shadow_observations_export_import_is_typed_and_does_not_store_raw_queries() -> TestResult {
    let source = SqliteStore::in_memory()?;
    LearnedSparseObservationRepository::append_observation(&source, observation(3))?;
    let exported = source.export_learned_sparse_observations()?;
    assert!(!exported.contains("alpha"));

    let imported = SqliteStore::in_memory()?;
    imported.import_learned_sparse_observations(&exported)?;
    let stored = LearnedSparseObservationRepository::scan_observations(&imported, limit(1)?)?;
    assert_eq!(stored, vec![observation(3)]);
    Ok(())
}

#[test]
fn shadow_observation_import_rejects_invalid_schema_without_touching_events() -> TestResult {
    let store = SqliteStore::in_memory()?;
    let event = super::registered(1, 1, 1);
    EventLog::append(&store, event.clone())?;
    let invalid = r#"[{
        "schema_version": 2,
        "query_id": 1,
        "query_class": "ExactLiteral",
        "route": "Shadow",
        "corpus_snapshot": 7,
        "index_generation": 11,
        "elapsed_ms": 3,
        "lanes": []
    }]"#;
    assert!(store.import_learned_sparse_observations(invalid).is_err());
    assert_eq!(
        EventLog::scan(&store, maestria_ports::EventFilter { artifact_id: None })?,
        vec![event]
    );
    Ok(())
}
