use crate::FullTextIndex;
use crate::in_memory::InMemoryFullTextIndex;
use maestria_domain::{ArtifactId, ChunkId};

#[test]
fn reindex_chunk_replaces_old_record() -> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryFullTextIndex::new();
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);

    index.index_chunks(vec![crate::IndexedChunk {
        artifact_id,
        chunk_id,
        text: "original text".to_string(),
    }])?;

    index.index_chunks(vec![crate::IndexedChunk {
        artifact_id,
        chunk_id,
        text: "updated text".to_string(),
    }])?;

    let hits = index.search(crate::SearchQuery {
        q: "original".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert!(
        hits.is_empty(),
        "old text must not be searchable after replacement"
    );

    let updated_hits = index.search(crate::SearchQuery {
        q: "updated".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(updated_hits.len(), 1);
    assert_eq!(updated_hits[0].chunk.text, "updated text");
    Ok(())
}

#[test]
fn reindex_chunk_preserves_other_chunks() -> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryFullTextIndex::new();
    let artifact_id = ArtifactId::new(1);

    index.index_chunks(vec![
        crate::IndexedChunk {
            artifact_id,
            chunk_id: ChunkId::new(10),
            text: "alpha".to_string(),
        },
        crate::IndexedChunk {
            artifact_id,
            chunk_id: ChunkId::new(11),
            text: "beta".to_string(),
        },
    ])?;

    index.index_chunks(vec![crate::IndexedChunk {
        artifact_id,
        chunk_id: ChunkId::new(10),
        text: "alpha updated".to_string(),
    }])?;

    let all_hits = index.search(crate::SearchQuery {
        q: "alpha".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(all_hits.len(), 1);
    assert_eq!(all_hits[0].chunk.chunk_id, ChunkId::new(10));

    let beta_hits = index.search(crate::SearchQuery {
        q: "beta".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(beta_hits.len(), 1);
    assert_eq!(beta_hits[0].chunk.chunk_id, ChunkId::new(11));
    Ok(())
}

#[test]
fn blank_chunk_search_query_returns_typed_error() -> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryFullTextIndex::new();

    for blank in ["", " ", "   ", "\t", "\n"] {
        let err = match index.search(crate::SearchQuery {
            q: blank.to_string(),
            limit: 10,
            offset: 0,
        }) {
            Ok(_) => return Err("blank chunk query unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(
            err.is_invalid_input(),
            "expected invalid input for {:?}, got {:?}",
            blank,
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("empty chunk search query"),
            "expected 'empty chunk search query' in error, got: {}",
            msg
        );
    }
    Ok(())
}

#[test]
fn blank_card_search_query_returns_typed_error() -> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryFullTextIndex::new();

    for blank in ["", " ", "   ", "\t", "\n"] {
        let err = match index.search_cards(crate::SearchQuery {
            q: blank.to_string(),
            limit: 10,
            offset: 0,
        }) {
            Ok(_) => return Err("blank card query unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(
            err.is_invalid_input(),
            "expected invalid input for {:?}, got {:?}",
            blank,
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("empty card search query"),
            "expected 'empty card search query' in error, got: {}",
            msg
        );
    }
    Ok(())
}

#[test]
fn blank_filtered_chunk_search_query_returns_typed_error() -> Result<(), Box<dyn std::error::Error>>
{
    let index = InMemoryFullTextIndex::new();

    let err = match index.search_filtered(
        crate::SearchQuery {
            q: "   ".to_string(),
            limit: 10,
            offset: 0,
        },
        &|_, _| true,
    ) {
        Ok(_) => return Err("blank filtered chunk query unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert!(err.is_invalid_input());
    let msg = err.to_string();
    assert!(
        msg.contains("empty filtered chunk search query"),
        "expected 'empty filtered chunk search query' in error, got: {}",
        msg
    );
    Ok(())
}

#[test]
fn blank_filtered_card_search_query_returns_typed_error() -> Result<(), Box<dyn std::error::Error>>
{
    let index = InMemoryFullTextIndex::new();

    let err = match index.search_cards_filtered(
        crate::SearchQuery {
            q: "   ".to_string(),
            limit: 10,
            offset: 0,
        },
        &|_, _| true,
    ) {
        Ok(_) => return Err("blank filtered card query unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert!(err.is_invalid_input());
    let msg = err.to_string();
    assert!(
        msg.contains("empty filtered card search query"),
        "expected 'empty filtered card search query' in error, got: {}",
        msg
    );
    Ok(())
}
