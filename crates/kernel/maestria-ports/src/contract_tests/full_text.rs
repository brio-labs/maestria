use super::*;
use maestria_domain::{ArtifactId, CardId, ChunkId};

use super::fixtures::*;

pub fn assert_full_text_index_round_trip(
    index: &impl FullTextIndex,
) -> Result<(), Box<dyn std::error::Error>> {
    verify_chunk_round_trip(index)?;
    verify_card_round_trip(index)?;
    verify_card_replacement(index)?;
    verify_tie_ordering(index)?;
    verify_empty_query(index)?;
    verify_artifact_chunk_batch(index)?;
    Ok(())
}

/// One `index_artifact_chunk` update must make the chunk and its cards
/// searchable together, and a re-index with updated content must replace
/// the prior documents (idempotent delete-then-add).
///
/// Query terms are invented words so the assertions cannot collide with any
/// document written by the earlier contract steps (the in-memory adapter
/// matches case-insensitive substrings).
fn verify_artifact_chunk_batch(
    index: &impl FullTextIndex,
) -> Result<(), Box<dyn std::error::Error>> {
    index.index_artifact_chunk(
        IndexedChunk {
            artifact_id: ArtifactId::new(7),
            chunk_id: ChunkId::new(70),
            text: "zymurgy quuxwobble".to_string(),
        },
        vec![IndexedCard {
            artifact_id: ArtifactId::new(7),
            card_id: CardId::new(700),
            title: "Zymurgy Card".to_string(),
            body: "zymurgy frazzle".to_string(),
        }],
        None,
        Vec::new(),
    )?;

    let chunk_hits = index.search(SearchQuery {
        q: "quuxwobble".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(chunk_hits.hits.len(), 1);
    assert_eq!(chunk_hits.hits[0].chunk.chunk_id, ChunkId::new(70));

    let card_hits = index.search_cards(SearchQuery {
        q: "frazzle".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(card_hits.hits.len(), 1);
    assert_eq!(card_hits.hits[0].card.card_id, CardId::new(700));

    // Re-index the same chunk with updated content: the old document must be
    // replaced, not duplicated.
    index.index_artifact_chunk(
        IndexedChunk {
            artifact_id: ArtifactId::new(7),
            chunk_id: ChunkId::new(70),
            text: "zymurgy quuxwobble revv".to_string(),
        },
        Vec::new(),
        None,
        Vec::new(),
    )?;
    let revised_hits = index.search(SearchQuery {
        q: "revv".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(revised_hits.hits.len(), 1);
    assert_eq!(revised_hits.hits[0].chunk.chunk_id, ChunkId::new(70));
    assert_eq!(revised_hits.hits[0].chunk.text, "zymurgy quuxwobble revv");
    Ok(())
}

fn verify_chunk_round_trip(index: &impl FullTextIndex) -> Result<(), Box<dyn std::error::Error>> {
    // --- chunk round-trip (existing) ---
    index.index_chunks(vec![
        IndexedChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
            text: "hello short".to_string(),
        },
        IndexedChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(11),
            text: "hello search with more ranking text".to_string(),
        },
        IndexedChunk {
            artifact_id: ArtifactId::new(2),
            chunk_id: ChunkId::new(20),
            text: "unrelated".to_string(),
        },
    ])?;

    let hits = index.search(SearchQuery {
        q: "hello".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert_eq!(hits.hits.len(), 2);
    let hit_ids: Vec<ChunkId> = hits.hits.iter().map(|hit| hit.chunk.chunk_id).collect();
    assert_eq!(hits.execution.budget.max_results(), 10);
    assert_eq!(
        hits.execution.completion,
        maestria_domain::SearchExecutionCompletion::Complete
    );
    assert!(hit_ids.contains(&ChunkId::new(10)));
    assert!(hit_ids.contains(&ChunkId::new(11)));
    let repeated = index.search(SearchQuery {
        q: "hello".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(hits, repeated);
    Ok(())
}

fn verify_card_round_trip(index: &impl FullTextIndex) -> Result<(), Box<dyn std::error::Error>> {
    // --- card round-trip ---
    index.index_cards(vec![
        IndexedCard {
            artifact_id: ArtifactId::new(1),
            card_id: CardId::new(100),
            title: "Alpha".to_string(),
            body: "first card".to_string(),
        },
        IndexedCard {
            artifact_id: ArtifactId::new(1),
            card_id: CardId::new(101),
            title: "Beta".to_string(),
            body: "second card with more content".to_string(),
        },
        IndexedCard {
            artifact_id: ArtifactId::new(2),
            card_id: CardId::new(200),
            title: "Gamma".to_string(),
            body: "unrelated".to_string(),
        },
    ])?;

    let card_hits = index.search_cards(SearchQuery {
        q: "card".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    assert_eq!(card_hits.hits.len(), 2);
    let card_ids: Vec<CardId> = card_hits.hits.iter().map(|hit| hit.card.card_id).collect();
    assert!(card_ids.contains(&CardId::new(100)));
    assert!(card_ids.contains(&CardId::new(101)));
    let repeated = index.search_cards(SearchQuery {
        q: "card".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(card_hits, repeated);
    Ok(())
}

fn verify_card_replacement(index: &impl FullTextIndex) -> Result<(), Box<dyn std::error::Error>> {
    // --- card replacement: re-index card 100 with updated content ---
    index.index_cards(vec![IndexedCard {
        artifact_id: ArtifactId::new(1),
        card_id: CardId::new(100),
        title: "Alpha Updated".to_string(),
        body: "revised first card".to_string(),
    }])?;

    // Old Beta (card_id=101) should still exist — only card 100 was re-indexed.
    let beta_hits = index.search_cards(SearchQuery {
        q: "second".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(beta_hits.hits.len(), 1);
    assert_eq!(beta_hits.hits[0].card.card_id, CardId::new(101));

    let updated_hits = index.search_cards(SearchQuery {
        q: "revised".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert_eq!(updated_hits.hits.len(), 1);
    assert_eq!(updated_hits.hits[0].card.card_id, CardId::new(100));
    assert_eq!(updated_hits.hits[0].card.title, "Alpha Updated");
    Ok(())
}

fn verify_tie_ordering(index: &impl FullTextIndex) -> Result<(), Box<dyn std::error::Error>> {
    // --- deterministic tie ordering: same scores, ordered by (artifact_id, card_id) ---
    index.index_cards(vec![
        IndexedCard {
            artifact_id: ArtifactId::new(3),
            card_id: CardId::new(301),
            title: "dup".to_string(),
            body: "same".to_string(),
        },
        IndexedCard {
            artifact_id: ArtifactId::new(3),
            card_id: CardId::new(302),
            title: "dup".to_string(),
            body: "same".to_string(),
        },
        IndexedCard {
            artifact_id: ArtifactId::new(3),
            card_id: CardId::new(303),
            title: "dup".to_string(),
            body: "same".to_string(),
        },
    ])?;

    let tie_hits = index.search_cards(SearchQuery {
        q: "dup".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;

    // All three should match; order must be by ascending card_id for ties
    let tie_ids: Vec<CardId> = tie_hits.hits.iter().map(|h| h.card.card_id).collect();
    assert_eq!(
        tie_ids,
        vec![CardId::new(301), CardId::new(302), CardId::new(303)]
    );
    Ok(())
}

fn verify_empty_query(index: &impl FullTextIndex) -> Result<(), Box<dyn std::error::Error>> {
    // --- empty query returns empty ---
    let empty = index.search_cards(SearchQuery {
        q: "zzz_no_match".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: search_budget(10)?,
    })?;
    assert!(empty.hits.is_empty());
    Ok(())
}
