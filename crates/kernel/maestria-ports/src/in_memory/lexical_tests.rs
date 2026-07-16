use super::*;
use crate::FullTextIndex;
use crate::in_memory::InMemoryFullTextIndex;
use crate::lexical::{CardField, ChunkField, FieldSelector, HitReason, LexicalQuery, MatchMode};
use maestria_domain::{ArtifactId, CardId, ChunkId};

#[test]
fn test_search_lexical_exact_and_boosts() {
    let index = InMemoryFullTextIndex::new();
    let chunks = vec![
        IndexedLexicalChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
            text: "alpha beta gamma".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
        IndexedLexicalChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(11),
            text: "alpha".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
    ];
    index
        .index_lexical_chunks(chunks)
        .expect("index lexical chunks");

    let exact_query = LexicalQuery {
        q: "alpha".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Exact,
        fields: vec![FieldSelector {
            field: ChunkField::Text,
            boost: 1.0,
        }],
    };
    let hits = index
        .search_lexical(exact_query)
        .expect("search exact lexical chunks");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(11));
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::ExactMatch {
            field: "text".into()
        }
    );

    let contains_query = LexicalQuery {
        q: "alpha".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Contains,
        fields: vec![FieldSelector {
            field: ChunkField::Text,
            boost: 2.5,
        }],
    };
    let hits2 = index
        .search_lexical(contains_query)
        .expect("search contains lexical chunks");
    assert_eq!(hits2.len(), 2);
    assert_eq!(hits2[0].chunk.chunk_id, ChunkId::new(10));
    assert_eq!(hits2[1].chunk.chunk_id, ChunkId::new(11));
    assert_eq!(hits2[0].metadata.raw_score, 40.0);
    assert_eq!(
        hits2[0].metadata.reason,
        HitReason::FieldMatch {
            field: "text".into()
        }
    );
    assert_eq!(hits2[0].metadata.raw_rank, 1);
    assert_eq!(hits2[1].metadata.raw_rank, 2);
}

#[test]
fn test_search_cards_lexical_fields_and_boosts() {
    let index = InMemoryFullTextIndex::new();
    let cards = vec![
        IndexedLexicalCard {
            artifact_id: ArtifactId::new(2),
            card_id: CardId::new(20),
            title: "Rust".to_string(),
            body: "A safe systems programming language".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
        IndexedLexicalCard {
            artifact_id: ArtifactId::new(2),
            card_id: CardId::new(21),
            title: "Programming in Rust".to_string(),
            body: "Rust".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
    ];
    index
        .index_lexical_cards(cards)
        .expect("index lexical cards");

    let query = LexicalQuery {
        q: "Rust".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Exact,
        fields: vec![
            FieldSelector {
                field: CardField::Title,
                boost: 10.0,
            },
            FieldSelector {
                field: CardField::Body,
                boost: 1.0,
            },
        ],
    };
    let hits = index
        .search_cards_lexical(query)
        .expect("search lexical cards");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].card.card_id, CardId::new(20));
    assert_eq!(hits[0].metadata.raw_score, 40.0);
    assert_eq!(
        hits[0].metadata.reason,
        HitReason::ExactMatch {
            field: "title".into()
        }
    );
    assert_eq!(hits[1].card.card_id, CardId::new(21));
    assert_eq!(hits[1].metadata.raw_score, 4.0);
    assert_eq!(
        hits[1].metadata.reason,
        HitReason::ExactMatch {
            field: "body".into()
        }
    );
}

#[test]
fn test_search_lexical_filtered_acl() {
    let index = InMemoryFullTextIndex::new();
    let chunks = vec![
        IndexedLexicalChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
            text: "secret code".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
        IndexedLexicalChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(11),
            text: "public code".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
    ];
    index
        .index_lexical_chunks(chunks)
        .expect("index lexical chunks");
    let query = LexicalQuery {
        q: "code".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Contains,
        fields: vec![FieldSelector {
            field: ChunkField::Text,
            boost: 1.0,
        }],
    };
    let hits = index
        .search_lexical_filtered(query, &|chunk_id, _| chunk_id == ChunkId::new(11))
        .expect("search filtered lexical chunks");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(11));
}

#[test]
fn test_search_cards_lexical_filtered_acl() {
    let index = InMemoryFullTextIndex::new();
    let cards = vec![
        IndexedLexicalCard {
            artifact_id: ArtifactId::new(2),
            card_id: CardId::new(20),
            title: "Confidential Project".to_string(),
            body: "super secret plan".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
        IndexedLexicalCard {
            artifact_id: ArtifactId::new(2),
            card_id: CardId::new(21),
            title: "Public Project".to_string(),
            body: "open source plan".to_string(),
            path: None,
            filename: None,
            symbol: None,
        },
    ];
    index
        .index_lexical_cards(cards)
        .expect("index lexical cards");
    let query = LexicalQuery {
        q: "plan".to_string(),
        limit: 10,
        offset: 0,
        mode: MatchMode::Contains,
        fields: vec![FieldSelector {
            field: CardField::Body,
            boost: 1.0,
        }],
    };
    let hits = index
        .search_cards_lexical_filtered(query, &|card_id, _| card_id == CardId::new(21))
        .expect("search filtered lexical cards");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.card_id, CardId::new(21));
}
