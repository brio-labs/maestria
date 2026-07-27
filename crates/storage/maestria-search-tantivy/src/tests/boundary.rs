use crate::TantivyFullTextIndex;
use maestria_domain::{ArtifactId, ChunkId};
use maestria_ports::{FullTextIndex, SearchQuery};

fn chunk(artifact_id: u64, chunk_id: u64, text: &str) -> maestria_ports::IndexedChunk {
    maestria_ports::IndexedChunk {
        artifact_id: ArtifactId::new(artifact_id),
        chunk_id: ChunkId::new(chunk_id),
        text: text.to_string(),
    }
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
