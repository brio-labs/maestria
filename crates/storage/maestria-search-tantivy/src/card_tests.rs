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
fn card_search_returns_indexed_card_metadata() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_cards(vec![
            card(
                7,
                70,
                "Alpha Summary",
                "This card discusses alpha concepts.",
            ),
            card(8, 80, "Beta Overview", "Unrelated beta card body text."),
        ])
        .expect("index cards");

    let hits = index
        .search_cards(SearchQuery {
            q: "alpha".to_string(),
            limit: 10,
        })
        .expect("search cards");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.artifact_id, ArtifactId::new(7));
    assert_eq!(hits[0].card.card_id, CardId::new(70));
    assert_eq!(hits[0].card.title, "Alpha Summary");
    assert_eq!(hits[0].card.body, "This card discusses alpha concepts.");
    assert!(hits[0].score > 0);
}

#[test]
fn card_search_limit_is_honored() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_cards(vec![
            card(1, 10, "Shared Term One", "body one"),
            card(1, 11, "Shared Term Two", "body two"),
            card(1, 12, "Shared Term Three", "body three"),
        ])
        .expect("index cards");

    let hits = index
        .search_cards(SearchQuery {
            q: "shared".to_string(),
            limit: 2,
        })
        .expect("search cards");

    assert_eq!(hits.len(), 2);
}

#[test]
fn card_empty_query_is_invalid() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    let result = index.search_cards(SearchQuery {
        q: "  \t  ".to_string(),
        limit: 10,
    });

    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
}

#[test]
fn card_zero_limit_returns_empty() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_cards(vec![card(3, 30, "Present", "words only")])
        .expect("index cards");

    let hits = index
        .search_cards(SearchQuery {
            q: "present".to_string(),
            limit: 0,
        })
        .expect("search cards");

    assert!(hits.is_empty());
}

#[test]
fn card_reindexing_replaces_without_duplicates() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_cards(vec![card(
            2,
            20,
            "Original Title",
            "original searchable body",
        )])
        .expect("index original card");
    index
        .index_cards(vec![card(
            2,
            20,
            "Updated Title",
            "updated searchable body",
        )])
        .expect("reindex card");

    let hits = index
        .search_cards(SearchQuery {
            q: "searchable".to_string(),
            limit: 10,
        })
        .expect("search cards");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.artifact_id, ArtifactId::new(2));
    assert_eq!(hits[0].card.card_id, CardId::new(20));
    assert_eq!(hits[0].card.title, "Updated Title");
    assert_eq!(hits[0].card.body, "updated searchable body");
}

#[test]
fn card_no_results_for_missing_term() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_cards(vec![card(3, 30, "Present Title", "present body words")])
        .expect("index cards");

    let hits = index
        .search_cards(SearchQuery {
            q: "absent".to_string(),
            limit: 10,
        })
        .expect("search cards");

    assert!(hits.is_empty());
}

#[test]
fn card_directory_backed_index_can_be_reopened() {
    let directory = tempfile::TempDir::new().expect("create temp directory");
    let index = TantivyFullTextIndex::open(directory.path()).expect("open directory index");
    index
        .index_cards(vec![card(
            4,
            40,
            "Durable Card",
            "durable indexed card body",
        )])
        .expect("index card");
    drop(index);

    let reopened = TantivyFullTextIndex::open(directory.path()).expect("reopen directory index");
    let hits = reopened
        .search_cards(SearchQuery {
            q: "durable".to_string(),
            limit: 10,
        })
        .expect("search cards");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.artifact_id, ArtifactId::new(4));
    assert_eq!(hits[0].card.card_id, CardId::new(40));
}

#[test]
fn card_and_chunk_searches_are_isolated() {
    let index = TantivyFullTextIndex::in_memory().expect("create in-memory index");

    index
        .index_chunks(vec![IndexedChunk {
            artifact_id: ArtifactId::new(5),
            chunk_id: ChunkId::new(50),
            text: "unique chunk text".to_string(),
        }])
        .expect("index chunk");
    index
        .index_cards(vec![card(5, 50, "Unique Card", "unique card text")])
        .expect("index card");

    let card_hits = index
        .search_cards(SearchQuery {
            q: "unique".to_string(),
            limit: 10,
        })
        .expect("search cards");
    assert_eq!(card_hits.len(), 1);
    assert_eq!(card_hits[0].card.card_id, CardId::new(50));

    let chunk_hits = index
        .search(SearchQuery {
            q: "unique".to_string(),
            limit: 10,
        })
        .expect("search chunks");
    assert_eq!(chunk_hits.len(), 1);
    assert_eq!(chunk_hits[0].chunk.chunk_id, ChunkId::new(50));
}
