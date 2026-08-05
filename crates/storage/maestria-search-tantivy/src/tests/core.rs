use crate::TantivyFullTextIndex;
use maestria_domain::{ArtifactId, ChunkId, SearchExecutionBudget};
use maestria_ports::{FullTextIndex, IndexedChunk, SearchQuery};
use tempfile::TempDir;

fn search_budget(
    limit: u64,
) -> Result<maestria_domain::SearchExecutionBudget, maestria_domain::SearchCompatibilityError> {
    SearchExecutionBudget::new(limit, 10_000, 100_000, 0)
}

fn chunk(artifact_id: u64, chunk_id: u64, text: &str) -> IndexedChunk {
    IndexedChunk {
        artifact_id: ArtifactId::new(artifact_id),
        chunk_id: ChunkId::new(chunk_id),
        text: text.to_string(),
    }
}

#[test]
fn index_search_returns_source_openable_chunk_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_chunks(vec![
        chunk(7, 70, "alpha source chunk"),
        chunk(8, 80, "beta unrelated chunk"),
    ])?;

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
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_chunks(vec![
        chunk(1, 10, "shared term one"),
        chunk(1, 11, "shared term two"),
        chunk(1, 12, "shared term three"),
    ])?;

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
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![chunk(9, 90, "a payload larger than one byte")])?;
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
        maestria_domain::SearchExecutionCompletion::Exhausted(
            maestria_domain::SearchExecutionResource::BytesRead
        )
    );
    assert_eq!(hits.execution.usage.bytes_read, 0);
    Ok(())
}

#[test]
fn scorer_work_budget_exhaustion_is_reported_before_scoring()
-> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![
        chunk(1, 10, "shared term one"),
        chunk(1, 11, "shared term two"),
    ])?;
    let hits = index.search(SearchQuery {
        q: "shared".to_string(),
        limit: 2,
        offset: 0,
        execution_budget: SearchExecutionBudget::new(2, 10, 1, 0)?,
    })?;
    assert!(hits.hits.is_empty());
    assert_eq!(
        hits.execution.completion,
        maestria_domain::SearchExecutionCompletion::Exhausted(
            maestria_domain::SearchExecutionResource::WorkUnits
        )
    );
    assert_eq!(hits.execution.usage.work_units, 1);
    Ok(())
}

#[test]
fn filtered_search_excludes_denied_chunk_before_scoring() -> Result<(), Box<dyn std::error::Error>>
{
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![
        chunk(1, 10, "shared searchable term"),
        chunk(1, 11, "shared searchable term"),
    ])?;

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
fn filtered_authorization_accounts_indexed_identity_bytes() -> Result<(), Box<dyn std::error::Error>>
{
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![chunk(1, 10, "searchable term")])?;
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
        maestria_domain::SearchExecutionCompletion::Exhausted(
            maestria_domain::SearchExecutionResource::BytesRead
        )
    );
    assert_eq!(hits.execution.usage.bytes_read, 0);
    Ok(())
}

#[test]
fn empty_query_is_invalid() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

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
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_chunks(vec![chunk(2, 20, "original searchable text")])?;
    index.index_chunks(vec![chunk(2, 20, "updated searchable text")])?;

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
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_chunks(vec![chunk(3, 30, "present words only")])?;

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
