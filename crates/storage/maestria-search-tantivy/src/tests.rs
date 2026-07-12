use super::*;

use maestria_ports::SearchQuery;
use tempfile::TempDir;

fn chunk(artifact_id: u64, chunk_id: u64, text: &str) -> IndexedChunk {
    IndexedChunk {
        artifact_id: ArtifactId::new(artifact_id),
        chunk_id: ChunkId::new(chunk_id),
        text: text.to_string(),
    }
}

#[test]
fn index_search_returns_source_openable_chunk_metadata() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_chunks(vec![
            chunk(7, 70, "alpha source chunk"),
            chunk(8, 80, "beta unrelated chunk"),
        ])
        .expect("index chunks");

    let hits = index
        .search(SearchQuery {
            q: "alpha".to_string(),
            limit: 10,
        })
        .expect("search chunks");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(7));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(70));
    assert_eq!(hits[0].chunk.text, "alpha source chunk");
    assert!(hits[0].score > 0);
}

#[test]
fn limit_is_honored() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_chunks(vec![
            chunk(1, 10, "shared term one"),
            chunk(1, 11, "shared term two"),
            chunk(1, 12, "shared term three"),
        ])
        .expect("index chunks");

    let hits = index
        .search(SearchQuery {
            q: "shared".to_string(),
            limit: 2,
        })
        .expect("search chunks");

    assert_eq!(hits.len(), 2);
}

#[test]
fn empty_query_is_invalid() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    let result = index.search(SearchQuery {
        q: "  \t  ".to_string(),
        limit: 10,
    });

    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
}

#[test]
fn reindexing_same_chunk_replaces_without_duplicate_hits() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_chunks(vec![chunk(2, 20, "original searchable text")])
        .expect("index original chunk");
    index
        .index_chunks(vec![chunk(2, 20, "updated searchable text")])
        .expect("reindex chunk");

    let hits = index
        .search(SearchQuery {
            q: "searchable".to_string(),
            limit: 10,
        })
        .expect("search chunks");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(2));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(20));
    assert_eq!(hits[0].chunk.text, "updated searchable text");
}

#[test]
fn no_results_for_missing_term() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_chunks(vec![chunk(3, 30, "present words only")])
        .expect("index chunks");

    let hits = index
        .search(SearchQuery {
            q: "absent".to_string(),
            limit: 10,
        })
        .expect("search chunks");

    assert!(hits.is_empty());
}

#[test]
fn directory_backed_index_can_be_reopened() {
    let directory = TempDir::new().expect("create temp directory");
    let index = TantivyFullTextIndex::open(directory.path()).expect("open directory index");
    index
        .index_chunks(vec![chunk(4, 40, "durable indexed text")])
        .expect("index chunk");
    drop(index);

    let reopened = TantivyFullTextIndex::open(directory.path()).expect("reopen directory index");
    let hits = reopened
        .search(SearchQuery {
            q: "durable".to_string(),
            limit: 10,
        })
        .expect("search chunks");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(4));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(40));
}
