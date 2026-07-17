use super::*;

use maestria_ports::SearchQuery;

fn card(artifact_id: u64, card_id: u64, title: &str, body: &str) -> IndexedCard {
    IndexedCard {
        artifact_id: ArtifactId::new(artifact_id),
        card_id: CardId::new(card_id),
        title: title.to_string(),
        body: body.to_string(),
    }
}

#[test]
fn card_search_returns_indexed_card_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_cards(vec![
        card(
            7,
            70,
            "Alpha Summary",
            "This card discusses alpha concepts.",
        ),
        card(8, 80, "Beta Overview", "Unrelated beta card body text."),
    ])?;

    let hits = index.search_cards(SearchQuery {
        q: "alpha".to_string(),
        limit: 10,
        offset: 0,
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.artifact_id, ArtifactId::new(7));
    assert_eq!(hits[0].card.card_id, CardId::new(70));
    assert_eq!(hits[0].card.title, "Alpha Summary");
    assert_eq!(hits[0].card.body, "This card discusses alpha concepts.");
    assert!(hits[0].score > 0);
    Ok(())
}

#[test]
fn card_search_limit_is_honored() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_cards(vec![
        card(1, 10, "Shared Term One", "body one"),
        card(1, 11, "Shared Term Two", "body two"),
        card(1, 12, "Shared Term Three", "body three"),
    ])?;

    let hits = index.search_cards(SearchQuery {
        q: "shared".to_string(),
        limit: 2,
        offset: 0,
    })?;

    assert_eq!(hits.len(), 2);
    Ok(())
}

#[test]
fn card_empty_query_is_invalid() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    let result = index.search_cards(SearchQuery {
        q: "  \t  ".to_string(),
        limit: 10,
        offset: 0,
    });

    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}

#[test]
fn card_zero_limit_returns_empty() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_cards(vec![card(3, 30, "Present", "words only")])?;

    let hits = index.search_cards(SearchQuery {
        q: "present".to_string(),
        limit: 0,
        offset: 0,
    })?;

    assert!(hits.is_empty());
    Ok(())
}

#[test]
fn card_reindexing_replaces_without_duplicates() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_cards(vec![card(
        2,
        20,
        "Original Title",
        "original searchable body",
    )])?;
    index.index_cards(vec![card(
        2,
        20,
        "Updated Title",
        "updated searchable body",
    )])?;

    let hits = index.search_cards(SearchQuery {
        q: "searchable".to_string(),
        limit: 10,
        offset: 0,
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.artifact_id, ArtifactId::new(2));
    assert_eq!(hits[0].card.card_id, CardId::new(20));
    assert_eq!(hits[0].card.title, "Updated Title");
    assert_eq!(hits[0].card.body, "updated searchable body");
    Ok(())
}

#[test]
fn card_no_results_for_missing_term() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_cards(vec![card(3, 30, "Present Title", "present body words")])?;

    let hits = index.search_cards(SearchQuery {
        q: "absent".to_string(),
        limit: 10,
        offset: 0,
    })?;

    assert!(hits.is_empty());
    Ok(())
}

#[test]
fn card_directory_backed_index_can_be_reopened() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::TempDir::new()?;
    let index = TantivyFullTextIndex::open(directory.path())?;
    index.index_cards(vec![card(
        4,
        40,
        "Durable Card",
        "durable indexed card body",
    )])?;
    drop(index);

    let reopened = TantivyFullTextIndex::open(directory.path())?;
    let hits = reopened.search_cards(SearchQuery {
        q: "durable".to_string(),
        limit: 10,
        offset: 0,
    })?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.artifact_id, ArtifactId::new(4));
    assert_eq!(hits[0].card.card_id, CardId::new(40));
    Ok(())
}

#[test]
fn card_and_chunk_searches_are_isolated() -> Result<(), Box<dyn std::error::Error>> {
    let index = TantivyFullTextIndex::in_memory()?;

    index.index_chunks(vec![IndexedChunk {
        artifact_id: ArtifactId::new(5),
        chunk_id: ChunkId::new(50),
        text: "unique chunk text".to_string(),
    }])?;
    index.index_cards(vec![card(5, 50, "Unique Card", "unique card text")])?;

    let card_hits = index.search_cards(SearchQuery {
        q: "unique".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(card_hits.len(), 1);
    assert_eq!(card_hits[0].card.card_id, CardId::new(50));

    let chunk_hits = index.search(SearchQuery {
        q: "unique".to_string(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(chunk_hits.len(), 1);
    assert_eq!(chunk_hits[0].chunk.chunk_id, ChunkId::new(50));
    Ok(())
}
