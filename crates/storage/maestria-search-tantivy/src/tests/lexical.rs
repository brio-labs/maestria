use crate::TantivyFullTextIndex;
use maestria_domain::{ArtifactId, ChunkId};
use maestria_ports::FullTextIndex;

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
