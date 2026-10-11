use std::{io::Write, sync::Arc};

use crate::{
    RamMemoryBudget, TantivyFullTextIndex, ram_directory::BudgetedRamDirectory, test_ram_index,
};
use sillage_domain::{ArtifactId, ChunkId, SearchExecutionBudget};
use sillage_ports::{FullTextIndex, IndexedChunk, SearchQuery};
use sillage_test_support::search_budget;
use tantivy::directory::Directory;
use tempfile::TempDir;

fn chunk(artifact_id: u64, chunk_id: u64, text: &str) -> IndexedChunk {
    IndexedChunk {
        artifact_id: ArtifactId::new(artifact_id),
        chunk_id: ChunkId::new(chunk_id),
        text: text.to_string(),
    }
}

fn patterned_bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| seed.wrapping_add((offset % 251) as u8))
        .collect()
}

#[test]
fn index_search_returns_source_openable_chunk_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;

    index.index_chunks(vec![
        chunk(7, 70, "alpha source chunk"),
        chunk(8, 80, "beta unrelated chunk"),
    ])?;
    index.commit_and_reload()?;

    let hits = index.search(SearchQuery {
        q: "alpha".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].chunk.artifact_id, ArtifactId::new(7));
    assert_eq!(hits.hits[0].chunk.chunk_id, ChunkId::new(70));
    assert_eq!(hits.hits[0].chunk.text, "alpha source chunk");
    assert!(hits.hits[0].score > 0);
    Ok(())
}

#[test]
fn limit_is_honored() -> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;

    index.index_chunks(vec![
        chunk(1, 10, "shared term one"),
        chunk(1, 11, "shared term two"),
        chunk(1, 12, "shared term three"),
    ])?;
    index.commit_and_reload()?;

    let hits = index.search(SearchQuery {
        q: "shared".to_string(),
        limit: 2,
        offset: 0,
        execution_budget: search_budget(2)?,
    })?;

    assert_eq!(hits.hits.len(), 2);
    assert_eq!(hits.execution.budget.max_results(), 2);
    Ok(())
}

#[test]
fn byte_budget_exhaustion_is_reported_before_chunk_result() -> Result<(), Box<dyn std::error::Error>>
{
    let index = test_ram_index()?;
    index.index_chunks(vec![chunk(9, 90, "a payload larger than one byte")])?;
    index.commit_and_reload()?;
    let budget = SearchExecutionBudget::new(1, 10, 10, 1)?;

    let hits = index.search(SearchQuery {
        q: "payload".to_string(),
        limit: 1,
        offset: 0,
        execution_budget: budget,
    })?;

    assert!(hits.hits.is_empty());
    assert_eq!(
        hits.execution.completion,
        sillage_domain::SearchExecutionCompletion::Exhausted(
            sillage_domain::SearchExecutionResource::BytesRead
        )
    );
    assert_eq!(hits.execution.usage.bytes_read, 0);
    Ok(())
}

#[test]
fn scorer_work_budget_exhaustion_is_reported_before_scoring()
-> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;
    index.index_chunks(vec![
        chunk(1, 10, "shared term one"),
        chunk(1, 11, "shared term two"),
    ])?;
    index.commit_and_reload()?;
    let hits = index.search(SearchQuery {
        q: "shared".to_string(),
        limit: 2,
        offset: 0,
        execution_budget: SearchExecutionBudget::new(2, 10, 1, 0)?,
    })?;
    assert!(hits.hits.is_empty());
    assert_eq!(
        hits.execution.completion,
        sillage_domain::SearchExecutionCompletion::Exhausted(
            sillage_domain::SearchExecutionResource::WorkUnits
        )
    );
    assert_eq!(hits.execution.usage.work_units, 1);
    Ok(())
}

#[test]
fn filtered_search_excludes_denied_chunk_before_scoring() -> Result<(), Box<dyn std::error::Error>>
{
    let index = test_ram_index()?;
    index.index_chunks(vec![
        chunk(1, 10, "shared searchable term"),
        chunk(1, 11, "shared searchable term"),
    ])?;
    index.commit_and_reload()?;

    let hits = index.search_filtered(
        SearchQuery {
            q: "shared".to_string(),
            limit: 10,
            offset: 0,
            execution_budget: search_budget(10)?,
        },
        &|chunk_id, _| Ok(chunk_id == ChunkId::new(10)),
    )?;
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].chunk.chunk_id, ChunkId::new(10));
    Ok(())
}

#[test]
fn filtered_rare_query_does_not_spend_work_on_unrelated_sources()
-> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;
    let mut chunks: Vec<_> = (1..=100)
        .map(|id| chunk(id, id * 10, "ordinary ledger entry"))
        .collect();
    chunks.push(chunk(101, 1010, "distinctive cobalt kestrel passage"));
    index.index_chunks(chunks)?;
    index.commit_and_reload()?;

    let hits = index.search_filtered(
        SearchQuery {
            q: "distinctive".to_string(),
            limit: 1,
            offset: 0,
            execution_budget: SearchExecutionBudget::new(1, 10, 32, 8192)?,
        },
        &|_, artifact_id| Ok(artifact_id == ArtifactId::new(101)),
    )?;
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].chunk.artifact_id, ArtifactId::new(101));
    Ok(())
}

#[test]
fn filtered_authorization_accounts_indexed_identity_bytes() -> Result<(), Box<dyn std::error::Error>>
{
    let index = test_ram_index()?;
    index.index_chunks(vec![chunk(1, 10, "searchable term")])?;
    index.commit_and_reload()?;
    let hits = index.search_filtered(
        SearchQuery {
            q: "searchable".to_string(),
            limit: 1,
            offset: 0,
            execution_budget: SearchExecutionBudget::new(1, 10, 10, 15)?,
        },
        &|_, _| Ok(false),
    )?;
    assert!(hits.hits.is_empty());
    assert_eq!(
        hits.execution.completion,
        sillage_domain::SearchExecutionCompletion::Exhausted(
            sillage_domain::SearchExecutionResource::BytesRead
        )
    );
    assert_eq!(hits.execution.usage.bytes_read, 0);
    Ok(())
}

#[test]
fn empty_query_is_invalid() -> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;

    let result = index.search(SearchQuery {
        q: "  \t  ".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    });

    assert!(result.is_err_and(|error| error.is_invalid_input()));
    Ok(())
}

#[test]
fn reindexing_same_chunk_replaces_without_duplicate_hits() -> Result<(), Box<dyn std::error::Error>>
{
    let index = test_ram_index()?;

    index.index_chunks(vec![chunk(2, 20, "original searchable text")])?;
    index.index_chunks(vec![chunk(2, 20, "updated searchable text")])?;
    index.commit_and_reload()?;

    let hits = index.search(SearchQuery {
        q: "searchable".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].chunk.artifact_id, ArtifactId::new(2));
    assert_eq!(hits.hits[0].chunk.chunk_id, ChunkId::new(20));
    assert_eq!(hits.hits[0].chunk.text, "updated searchable text");
    Ok(())
}

#[test]
fn no_results_for_missing_term() -> Result<(), Box<dyn std::error::Error>> {
    let index = test_ram_index()?;

    index.index_chunks(vec![chunk(3, 30, "present words only")])?;
    index.commit_and_reload()?;

    let hits = index.search(SearchQuery {
        q: "absent".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert!(hits.hits.is_empty());
    Ok(())
}

#[test]
fn directory_backed_index_can_be_reopened() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let index = TantivyFullTextIndex::open(directory.path())?;
    index.index_chunks(vec![chunk(4, 40, "durable indexed text")])?;
    index.commit_and_reload()?;
    drop(index);

    let reopened = TantivyFullTextIndex::open(directory.path())?;
    let hits = reopened.search(SearchQuery {
        q: "durable".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].chunk.artifact_id, ArtifactId::new(4));
    assert_eq!(hits.hits[0].chunk.chunk_id, ChunkId::new(40));
    Ok(())
}

#[test]
fn read_only_directory_backed_index_searches_durable_chunks()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let index = TantivyFullTextIndex::open(directory.path())?;
    index.index_chunks(vec![chunk(4, 40, "durable indexed text")])?;
    index.commit_and_reload()?;
    drop(index);

    let reopened = TantivyFullTextIndex::open_read_only(directory.path())?;
    let hits = reopened.search(SearchQuery {
        q: "durable".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].chunk.artifact_id, ArtifactId::new(4));
    assert_eq!(hits.hits[0].chunk.chunk_id, ChunkId::new(40));
    Ok(())
}

#[test]
fn read_only_directory_backed_index_pre_filters_before_scoring()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let index = TantivyFullTextIndex::open(directory.path())?;
    index.index_chunks(vec![chunk(4, 40, "durable indexed text")])?;
    index.commit_and_reload()?;
    drop(index);

    let reopened = TantivyFullTextIndex::open_read_only(directory.path())?;
    let hits = reopened.search_filtered(
        SearchQuery {
            q: "durable".to_string(),
            limit: 10,
            offset: 0,
            execution_budget: search_budget(10)?,
        },
        &|chunk_id, artifact_id| {
            Ok(chunk_id == ChunkId::new(40) && artifact_id == ArtifactId::new(4))
        },
    )?;

    assert_eq!(hits.hits.len(), 1);
    Ok(())
}

#[test]
fn read_only_card_rebuild_refusal_preserves_pending_marker_and_flag()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let writable = TantivyFullTextIndex::open(directory.path())?;
    drop(writable);
    let marker = directory.path().join(".cards-rebuild");
    const MARKER_BYTES: &[u8] = b"pending card rebuild marker";
    std::fs::write(&marker, MARKER_BYTES)?;

    let readonly = TantivyFullTextIndex::open_read_only(directory.path())?;
    assert!(readonly.needs_card_rebuild()?);
    assert!(readonly.complete_card_rebuild().is_err());
    assert_eq!(std::fs::read(&marker)?.as_slice(), MARKER_BYTES);
    assert!(readonly.needs_card_rebuild()?);
    Ok(())
}

#[test]
fn budgeted_ram_directory_preserves_append_order_across_staging_chunks()
-> Result<(), Box<dyn std::error::Error>> {
    const CHUNK_BYTES: usize = 16 * 1024;
    let directory = BudgetedRamDirectory::new(Arc::new(RamMemoryBudget::new(2 * 1024 * 1024)))?;
    let path = std::path::Path::new("append-order");
    let first = patterned_bytes(7, CHUNK_BYTES + 7);
    let second = patterned_bytes(31, 2 * CHUNK_BYTES + 11);
    let third = patterned_bytes(73, CHUNK_BYTES + 5);
    let mut expected = Vec::with_capacity(first.len() + second.len() + third.len());
    expected.extend_from_slice(&first);
    expected.extend_from_slice(&second);
    expected.extend_from_slice(&third);

    let mut output = directory.open_write(path)?;
    output.write_all(&first)?;
    output.write_all(&second)?;
    output.write_all(&third)?;
    output.flush()?;
    drop(output);

    assert_eq!(directory.atomic_read(path)?, expected);
    Ok(())
}
