use std::sync::Arc;

use maestria_domain::{
    ContentHash, CorpusSnapshotId, IndexGenerationId, IndexLifecycle, RepresentationName,
    SearchExecutionBudget, SparseNamespace, TrustZone,
};
use maestria_ports::{
    LearnedSparseIndex, LearnedSparseProjectionLifecycle, SPARSE_REPRESENTATION_V1, SparseDocument,
    SparseFingerprint, SparseIdentity, SparseSearchQuery, SparseTermWeight, SparseVector,
};
use tempfile::tempdir;

use crate::{SqliteLearnedSparseIndex, SqliteStore};

use maestria_ports::InMemoryLearnedSparseProvider;
use maestria_ports::learned_sparse_contract_tests::assert_learned_sparse_index_contract;
fn hash(digit: char) -> Result<ContentHash, Box<dyn std::error::Error>> {
    Ok(ContentHash::new(format!(
        "sha256:{}",
        digit.to_string().repeat(64)
    ))?)
}

fn identity(generation: u64, instance: &str) -> Result<SparseIdentity, Box<dyn std::error::Error>> {
    Ok(SparseIdentity {
        generation_id: IndexGenerationId::new(generation),
        corpus_snapshot: CorpusSnapshotId::new(11),
        representation: RepresentationName::new(SPARSE_REPRESENTATION_V1),
        namespace: SparseNamespace::new(instance, TrustZone::Verified, SPARSE_REPRESENTATION_V1)?,
        fingerprint: SparseFingerprint {
            provider: "frozen-local-provider".to_string(),
            model: "frozen-sparse-model".to_string(),
            revision: "revision-1".to_string(),
            artifact_hash: hash('1')?,
            tokenizer_hash: hash('2')?,
            vocabulary_hash: hash('3')?,
            vocabulary_size: 65_536,
            term_namespace: "frozen-vocabulary-v1".to_string(),
            query_template_hash: hash('4')?,
            document_template_hash: hash('5')?,
            preprocessing_version: "preprocessing-v1".to_string(),
            weighting_version: "weighting-v1".to_string(),
            quantization: "f32".to_string(),
            pruning_threshold: 0.0,
            max_terms: 128,
        },
    })
}

fn document(
    identity: &SparseIdentity,
    chunk_id: u64,
    content_hash: ContentHash,
    weight: f32,
) -> Result<SparseDocument, Box<dyn std::error::Error>> {
    Ok(SparseDocument {
        chunk_id: maestria_domain::ChunkId::new(chunk_id),
        content_hash,
        vector: SparseVector::new(identity.clone(), vec![SparseTermWeight::new(7, weight)?])?,
    })
}

fn query(identity: &SparseIdentity) -> Result<SparseSearchQuery, Box<dyn std::error::Error>> {
    Ok(SparseSearchQuery {
        vector: SparseVector::new(identity.clone(), vec![SparseTermWeight::new(7, 1.0)?])?,
        limit: 4,
        max_contributions: 4,
        execution_budget: SearchExecutionBudget::new(4, 32, 128, 10_000)?,
    })
}

#[test]
fn projection_survives_restart_and_replaces_or_tombstones_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("sparse.sqlite");
    let sparse_identity = identity(21, "instance-a")?;
    let store = Arc::new(SqliteStore::open(&path)?);
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.index_documents(vec![document(&sparse_identity, 1, hash('6')?, 1.0)?])?;
    assert!(projection.search(query(&sparse_identity)?).is_err());
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    assert_eq!(projection.search(query(&sparse_identity)?)?.hits.len(), 1);
    projection.index_documents(vec![document(&sparse_identity, 1, hash('7')?, 3.0)?])?;
    projection.delete_chunks(&[maestria_domain::ChunkId::new(1)])?;
    assert!(projection.search(query(&sparse_identity)?)?.hits.is_empty());
    drop(projection);
    drop(store);

    let reopened = Arc::new(SqliteStore::open(&path)?);
    let reopened_projection =
        SqliteLearnedSparseIndex::new(Arc::clone(&reopened), sparse_identity.clone())?;
    assert_eq!(reopened_projection.lifecycle()?, IndexLifecycle::Shadow);
    assert!(
        reopened_projection
            .search(query(&sparse_identity)?)?
            .hits
            .is_empty()
    );
    reopened_projection.rebuild(vec![document(&sparse_identity, 2, hash('8')?, 2.0)?])?;
    assert_eq!(
        reopened_projection
            .search(query(&sparse_identity)?)?
            .hits
            .len(),
        1
    );
    Ok(())
}

#[test]
fn search_reports_candidate_budget_exhaustion_without_loading_the_corpus()
-> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(26, "instance-a")?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.index_documents(vec![
        document(&sparse_identity, 1, hash('1')?, 1.0)?,
        document(&sparse_identity, 2, hash('2')?, 1.0)?,
    ])?;
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    let mut bounded = query(&sparse_identity)?;
    bounded.execution_budget = SearchExecutionBudget::new(4, 1, 128, 10_000)?;
    let result = projection.search(bounded)?;
    assert_eq!(result.hits.len(), 1);
    assert_eq!(
        result.execution.completion,
        maestria_domain::SearchExecutionCompletion::Exhausted(
            maestria_domain::SearchExecutionResource::Candidates
        )
    );
    Ok(())
}

#[test]
fn search_stops_before_vector_materialization_when_byte_budget_is_exhausted()
-> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(26, "instance-byte-budget")?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.index_documents(vec![document(&sparse_identity, 1, hash('1')?, 1.0)?])?;
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    let mut bounded = query(&sparse_identity)?;
    bounded.execution_budget = SearchExecutionBudget::new(4, 32, 128, 1)?;
    let result = projection.search(bounded)?;
    assert!(result.hits.is_empty());
    assert_eq!(
        result.execution.completion,
        maestria_domain::SearchExecutionCompletion::Exhausted(
            maestria_domain::SearchExecutionResource::BytesRead
        )
    );
    assert_eq!(result.execution.usage.bytes_read, 0);
    Ok(())
}

#[test]
fn oversized_persisted_sparse_vectors_fail_before_decoding()
-> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(28, "instance-oversized")?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.index_documents(vec![document(&sparse_identity, 1, hash('1')?, 1.0)?])?;
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    let identity_json = serde_json::to_string(&sparse_identity)?;
    let oversized = "x".repeat(1_048_577);
    let connection = store.lock()?;
    connection.execute(
        "UPDATE learned_sparse_projection_documents
         SET vector_json = ?1 WHERE identity_json = ?2 AND chunk_id = 1",
        rusqlite::params![oversized, identity_json],
    )?;
    drop(connection);
    assert!(projection.search(query(&sparse_identity)?).is_err());
    Ok(())
}

#[test]
fn corrupted_projection_metadata_fails_closed_before_lifecycle_use()
-> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(27, "instance-a")?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    let other_namespace =
        SparseNamespace::new("instance-b", TrustZone::Verified, SPARSE_REPRESENTATION_V1)?;
    let namespace_json = serde_json::to_string(&other_namespace)?;
    let identity_json = serde_json::to_string(&sparse_identity)?;
    let connection = store.lock()?;
    connection.execute(
        "UPDATE learned_sparse_projections SET namespace_json = ?1 WHERE identity_json = ?2",
        rusqlite::params![namespace_json, identity_json],
    )?;
    drop(connection);
    assert!(projection.lifecycle().is_err());
    assert!(
        projection
            .transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)
            .is_err()
    );
    assert!(
        projection
            .index_documents(vec![document(&sparse_identity, 2, hash('9')?, 1.0,)?])
            .is_err()
    );
    Ok(())
}

#[test]
fn lifecycle_activation_retires_previous_generation_and_supports_rollback_and_collection()
-> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let first_identity = identity(31, "instance-a")?;
    let second_identity = identity(32, "instance-a")?;
    let first = SqliteLearnedSparseIndex::new(Arc::clone(&store), first_identity.clone())?;
    let second = SqliteLearnedSparseIndex::new(Arc::clone(&store), second_identity.clone())?;
    first.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    first.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    first.transition(IndexLifecycle::Shadow, IndexLifecycle::Active)?;
    second.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    second.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    second.transition(IndexLifecycle::Shadow, IndexLifecycle::Active)?;
    assert_eq!(first.lifecycle()?, IndexLifecycle::Retired);
    assert_eq!(second.lifecycle()?, IndexLifecycle::Active);
    first.transition(IndexLifecycle::Retired, IndexLifecycle::Active)?;
    assert_eq!(second.lifecycle()?, IndexLifecycle::Retired);
    first.transition(IndexLifecycle::Active, IndexLifecycle::Retired)?;
    first.transition(IndexLifecycle::Retired, IndexLifecycle::Collectable)?;
    first.collect()?;
    assert_eq!(first.lifecycle()?, IndexLifecycle::Tombstoned);
    Ok(())
}

#[test]
fn projection_passes_shared_sparse_index_contract_after_shadow_activation()
-> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(36, "instance-a")?;
    let provider = InMemoryLearnedSparseProvider::new(sparse_identity.clone())?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    assert_learned_sparse_index_contract(&projection, &provider)?;
    Ok(())
}

#[test]
fn generation_identity_collision_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let first = identity(41, "instance-a")?;
    let collision = identity(41, "instance-b")?;
    let _projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), first)?;
    assert!(SqliteLearnedSparseIndex::new(store, collision).is_err());
    Ok(())
}

#[test]
fn search_cache_invalidates_on_every_write() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(41, "instance-cache")?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.index_documents(vec![document(&sparse_identity, 1, hash('1')?, 1.0)?])?;
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    assert_eq!(projection.search(query(&sparse_identity)?)?.hits.len(), 1);
    // A write must invalidate the warm cache, not serve stale rows.
    projection.index_documents(vec![document(&sparse_identity, 2, hash('2')?, 2.0)?])?;
    assert_eq!(projection.search(query(&sparse_identity)?)?.hits.len(), 2);
    projection.delete_chunks(&[maestria_domain::ChunkId::new(1)])?;
    assert_eq!(projection.search(query(&sparse_identity)?)?.hits.len(), 1);
    projection.clear()?;
    assert!(projection.search(query(&sparse_identity)?)?.hits.is_empty());
    // Rebuild restores the projection and the cache follows.
    projection.rebuild(vec![document(&sparse_identity, 3, hash('3')?, 3.0)?])?;
    assert_eq!(projection.search(query(&sparse_identity)?)?.hits.len(), 1);
    Ok(())
}

#[test]
fn search_falls_back_to_cold_reads_without_a_version_row() -> Result<(), Box<dyn std::error::Error>>
{
    let store = Arc::new(SqliteStore::in_memory()?);
    let sparse_identity = identity(42, "instance-cache-fallback")?;
    let projection = SqliteLearnedSparseIndex::new(Arc::clone(&store), sparse_identity.clone())?;
    projection.index_documents(vec![document(&sparse_identity, 1, hash('1')?, 1.0)?])?;
    projection.transition(IndexLifecycle::Building, IndexLifecycle::Evaluated)?;
    projection.transition(IndexLifecycle::Evaluated, IndexLifecycle::Shadow)?;
    // A projection written before the version row: cold per-document reads.
    let connection = store.lock()?;
    connection
        .execute(
            "DELETE FROM learned_sparse_projection_meta WHERE identity_json = ?1",
            rusqlite::params![serde_json::to_string(&sparse_identity)?],
        )
        .map_err(|error| {
            crate::sqlite_store::to_port_error(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            ))
        })?;
    drop(connection);
    assert_eq!(projection.search(query(&sparse_identity)?)?.hits.len(), 1);
    Ok(())
}
