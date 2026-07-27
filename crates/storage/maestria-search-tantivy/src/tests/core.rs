use crate::TantivyFullTextIndex;
use maestria_domain::{ArtifactId, ChunkId};
use maestria_ports::{FullTextIndex, IndexedChunk, PortError, SearchQuery};
use tempfile::TempDir;

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
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(7));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(70));
    assert_eq!(hits[0].chunk.text, "alpha source chunk");
    assert!(hits[0].score > 0);
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
    })?;

    assert_eq!(hits.len(), 2);
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
        },
        &|chunk_id, _| chunk_id == ChunkId::new(10),
    )?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(10));
    Ok(())
}

#[test]
fn empty_query_is_invalid() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    let result = index.search(SearchQuery {
        q: "  \t  ".to_string(),
        limit: 10,
        offset: 0,
    });

    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
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
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(2));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(20));
    assert_eq!(hits[0].chunk.text, "updated searchable text");
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
    })?;

    assert!(hits.is_empty());
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
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(4));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(40));
    Ok(())
}
