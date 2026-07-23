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

#[test]
fn satisfies_shared_full_text_index_contract() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;
    assert_full_text_index_round_trip(&index)?;
    Ok(())
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
fn lexical_index_search_returns_chunk_metadata_contains_match()
-> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_lexical_chunks(vec![
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
    ])?;

    let hits = index.search_lexical(LexicalQuery {
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
    })?;

    assert_eq!(hits.len(), 2); // Both chunks match (chunk 1 text, chunk 2 path/filename/symbol)
    // Rank 1 will be chunk 1 (shorter text gives higher score in manual scoring) or chunk 2 (more matched fields? manual scoring only scores one field)
    // But the important part is both are present.
    Ok(())
}

#[test]
fn lexical_contains_matches_inside_symbol_tokens() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_lexical_chunks(vec![lexical_chunk(
        9,
        90,
        "unrelated",
        None,
        None,
        Some("Searcher"),
    )])?;

    let hits = index.search_lexical(LexicalQuery {
        q: "earch".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Contains,
        fields: vec![FieldSelector {
            field: ChunkField::Symbol,
            boost: 1.0,
        }],
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.symbol.as_deref(), Some("Searcher"));
    Ok(())
}

#[test]
fn lexical_index_search_exact_match() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_lexical_chunks(vec![
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
    ])?;

    let hits = index.search_lexical(LexicalQuery {
        q: "mod.rs".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Exact,
        fields: vec![FieldSelector {
            field: ChunkField::Filename,
            boost: 3.0,
        }],
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 1);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::ExactMatch {
            field: "filename".to_string()
        }
    );
    Ok(())
}

#[test]
fn lexical_index_search_exact_whole_field_text() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_lexical_chunks(vec![
        lexical_chunk(1, 10, "some exact text", None, None, None),
        lexical_chunk(2, 20, "exact text", None, None, None),
    ])?;

    // Searching for "exact text" should ONLY match chunk 2. Chunk 1 has extra text.
    let hits = index.search_lexical(LexicalQuery {
        q: "exact text".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Exact,
        fields: vec![FieldSelector {
            field: ChunkField::Text,
            boost: 1.0,
        }],
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 2);
    Ok(())
}

#[test]
fn lexical_index_search_metadata_contains() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_lexical_chunks(vec![lexical_chunk(
        1,
        10,
        "text",
        Some("src/module/sub/file.rs"),
        None,
        None,
    )])?;

    let hits = index.search_lexical(LexicalQuery {
        q: "module/sub".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Contains,
        fields: vec![FieldSelector {
            field: ChunkField::Path,
            boost: 1.0,
        }],
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 1);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::FieldMatch {
            field: "path".to_string()
        }
    );
    Ok(())
}

#[test]
fn lexical_search_honors_offset_and_rank() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_lexical_chunks(vec![
        lexical_chunk(1, 10, "alpha alpha", None, None, None),
        lexical_chunk(1, 11, "alpha", None, None, None),
    ])?;

    let hits = index.search_lexical(LexicalQuery {
        q: "alpha".to_string(),
        limit: 1,
        offset: 1,
        mode: MatchMode::Contains,
        fields: vec![FieldSelector {
            field: ChunkField::Text,
            boost: 1.0,
        }],
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(11));
    assert_eq!(hits[0].metadata.raw_rank, 2);
    Ok(())
}

#[test]
fn lexical_index_search_exact_id_match() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_ports::{ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_lexical_chunks(vec![
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
    ])?;

    let hits = index.search_lexical(LexicalQuery {
        q: "2:20".to_string(), // chunk_key(artifact_id, chunk_id)
        limit: 10,
        offset: 0,
        mode: MatchMode::Exact,
        fields: vec![FieldSelector {
            field: ChunkField::Id,
            boost: 3.0,
        }],
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.artifact_id.value(), 2);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::ExactMatch {
            field: "id".to_string()
        }
    );
    Ok(())
}

#[test]
fn very_large_query_still_parses() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![chunk(1, 10, "alpha beta gamma")])?;

    let long_term = "a".repeat(10_000);
    let hits = index.search(SearchQuery {
        q: long_term,
        limit: 10,
        offset: 0,
    })?;
    assert!(
        hits.is_empty(),
        "expected no hits for non-existent long term"
    );
    Ok(())
}

#[test]
fn special_character_injection_query_is_handled() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![chunk(1, 10, "alpha beta gamma")])?;

    // Tantivy query parser treats some of these as syntax.
    // The boundary requirement is that none of them panic or crash the process.
    for query in ["*", ":", "(", ")", "AND OR", "foo:"] {
        let _result = index.search(SearchQuery {
            q: query.to_string(),
            limit: 10,
            offset: 0,
        });
        // Reaching this line means no panic occurred, which is the boundary contract.
    }
    Ok(())
}

#[test]
fn unicode_boundary_query_works() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;
    index.index_chunks(vec![chunk(1, 10, "hello 世界"), chunk(2, 20, "alpha beta")])?;

    let hits = index.search(SearchQuery {
        q: "世界".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(10));
    Ok(())
}
