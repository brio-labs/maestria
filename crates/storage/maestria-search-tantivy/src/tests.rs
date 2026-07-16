use super::*;

use maestria_ports::{
    FullTextIndex, SearchQuery, contract_tests::assert_full_text_index_round_trip,
};
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
            offset: 0,
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
            offset: 0,
        })
        .expect("search chunks");

    assert_eq!(hits.len(), 2);
}

#[test]
fn filtered_search_excludes_denied_chunk_before_scoring() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");
    index
        .index_chunks(vec![
            chunk(1, 10, "shared searchable term"),
            chunk(1, 11, "shared searchable term"),
        ])
        .expect("index chunks");

    let hits = index
        .search_filtered(
            SearchQuery {
                q: "shared".to_string(),
                limit: 10,
                offset: 0,
            },
            &|chunk_id, _| chunk_id == ChunkId::new(10),
        )
        .expect("filtered search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(10));
}

#[test]
fn empty_query_is_invalid() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    let result = index.search(SearchQuery {
        q: "  \t  ".to_string(),
        limit: 10,
        offset: 0,
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
            offset: 0,
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
            offset: 0,
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
            offset: 0,
        })
        .expect("search chunks");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id, ArtifactId::new(4));
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(40));
}

#[test]
fn satisfies_shared_full_text_index_contract() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");
    assert_full_text_index_round_trip(&index);
}

fn lexical_chunk(
    artifact_id: u64,
    chunk_id: u64,
    text: &str,
    path: Option<&str>,
    filename: Option<&str>,
    symbol: Option<&str>,
) -> maestria_ports::IndexedLexicalChunk {
    maestria_ports::IndexedLexicalChunk {
        artifact_id: ArtifactId::new(artifact_id),
        chunk_id: ChunkId::new(chunk_id),
        text: text.to_string(),
        path: path.map(|s| s.to_string()),
        filename: filename.map(|s| s.to_string()),
        symbol: symbol.map(|s| s.to_string()),
    }
}

#[test]
fn lexical_index_search_returns_chunk_metadata_contains_match() {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_lexical_chunks(vec![
            lexical_chunk(
                1,
                10,
                "impl search interface",
                Some("src/search/mod.rs"),
                Some("mod.rs"),
                Some("Searcher"),
            ),
            lexical_chunk(
                2,
                20,
                "impl tantivy interface",
                Some("src/tantivy/search.rs"),
                Some("search.rs"),
                Some("TantivySearcher"),
            ),
        ])
        .expect("index lexical chunks");

    let hits = index
        .search_lexical(LexicalQuery {
            q: "search".to_string(),
            limit: 10,
            offset: 0,
            mode: MatchMode::Contains,
            fields: vec![
                FieldSelector {
                    field: ChunkField::Text,
                    boost: 1.0,
                },
                FieldSelector {
                    field: ChunkField::Path,
                    boost: 2.0,
                },
                FieldSelector {
                    field: ChunkField::Filename,
                    boost: 3.0,
                },
                FieldSelector {
                    field: ChunkField::Symbol,
                    boost: 4.0,
                },
            ],
        })
        .expect("search lexical");

    assert_eq!(hits.len(), 2); // Both chunks match (chunk 1 text, chunk 2 path/filename/symbol)
    // Rank 1 will be chunk 1 (shorter text gives higher score in manual scoring) or chunk 2 (more matched fields? manual scoring only scores one field)
    // But the important part is both are present.
}

#[test]
fn lexical_contains_matches_inside_symbol_tokens() {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");
    index
        .index_lexical_chunks(vec![lexical_chunk(
            9,
            90,
            "unrelated",
            None,
            None,
            Some("Searcher"),
        )])
        .expect("index lexical chunk");

    let hits = index
        .search_lexical(LexicalQuery {
            q: "earch".to_string(),
            limit: 10,
            offset: 0,
            mode: MatchMode::Contains,
            fields: vec![FieldSelector {
                field: ChunkField::Symbol,
                boost: 1.0,
            }],
        })
        .expect("search symbol substring");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.symbol.as_deref(), Some("Searcher"));
}

#[test]
fn lexical_index_search_exact_match() {
    use maestria_ports::{ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_lexical_chunks(vec![
            lexical_chunk(
                1,
                10,
                "some text",
                Some("src/search/mod.rs"),
                Some("mod.rs"),
                Some("Searcher"),
            ),
            lexical_chunk(
                2,
                20,
                "other text",
                Some("src/tantivy/search.rs"),
                Some("search.rs"),
                Some("TantivySearcher"),
            ),
        ])
        .expect("index lexical chunks");

    let hits = index
        .search_lexical(LexicalQuery {
            q: "mod.rs".to_string(),
            limit: 10,
            offset: 0,
            mode: MatchMode::Exact,
            fields: vec![FieldSelector {
                field: ChunkField::Filename,
                boost: 3.0,
            }],
        })
        .expect("search lexical exact");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 1);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::ExactMatch {
            field: "filename".to_string()
        }
    );
}

#[test]
fn lexical_index_search_exact_whole_field_text() {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_lexical_chunks(vec![
            lexical_chunk(1, 10, "some exact text", None, None, None),
            lexical_chunk(2, 20, "exact text", None, None, None),
        ])
        .expect("index lexical chunks");

    // Searching for "exact text" should ONLY match chunk 2. Chunk 1 has extra text.
    let hits = index
        .search_lexical(LexicalQuery {
            q: "exact text".to_string(),
            limit: 10,
            offset: 0,
            mode: MatchMode::Exact,
            fields: vec![FieldSelector {
                field: ChunkField::Text,
                boost: 1.0,
            }],
        })
        .expect("search lexical exact");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 2);
}

#[test]
fn lexical_index_search_metadata_contains() {
    use maestria_ports::{ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_lexical_chunks(vec![lexical_chunk(
            1,
            10,
            "text",
            Some("src/module/sub/file.rs"),
            None,
            None,
        )])
        .expect("index lexical chunks");

    let hits = index
        .search_lexical(LexicalQuery {
            q: "module/sub".to_string(),
            limit: 10,
            offset: 0,
            mode: MatchMode::Contains,
            fields: vec![FieldSelector {
                field: ChunkField::Path,
                boost: 1.0,
            }],
        })
        .expect("search lexical contains");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 1);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::FieldMatch {
            field: "path".to_string()
        }
    );
}

#[test]
fn lexical_search_honors_offset_and_rank() {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");
    index
        .index_lexical_chunks(vec![
            lexical_chunk(1, 10, "alpha alpha", None, None, None),
            lexical_chunk(1, 11, "alpha", None, None, None),
        ])
        .expect("index lexical chunks");

    let hits = index
        .search_lexical(LexicalQuery {
            q: "alpha".to_string(),
            limit: 1,
            offset: 1,
            mode: MatchMode::Contains,
            fields: vec![FieldSelector {
                field: ChunkField::Text,
                boost: 1.0,
            }],
        })
        .expect("search lexical page");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(11));
    assert_eq!(hits[0].metadata.raw_rank, 2);
}

#[test]
fn lexical_index_search_exact_id_match() {
    use maestria_ports::{ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_lexical_chunks(vec![
            lexical_chunk(
                1,
                10,
                "some text",
                Some("src/search/mod.rs"),
                Some("mod.rs"),
                Some("Searcher"),
            ),
            lexical_chunk(
                2,
                20,
                "other text",
                Some("src/tantivy/search.rs"),
                Some("search.rs"),
                Some("TantivySearcher"),
            ),
        ])
        .expect("index lexical chunks");

    let hits = index
        .search_lexical(LexicalQuery {
            q: "2:20".to_string(), // chunk_key(artifact_id, chunk_id)
            limit: 10,
            offset: 0,
            mode: MatchMode::Exact,
            fields: vec![FieldSelector {
                field: ChunkField::Id,
                boost: 3.0,
            }],
        })
        .expect("search lexical exact id");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 2);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::ExactMatch {
            field: "id".to_string()
        }
    );
}
