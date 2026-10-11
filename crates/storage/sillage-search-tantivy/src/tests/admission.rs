use std::sync::Arc;

use crate::{InMemoryIndexError, MemoryBudgetError, RamMemoryBudget, TantivyFullTextIndex};
use sillage_domain::{ArtifactId, CardId, ChunkId};
use sillage_ports::{
    FullTextIndex, IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk, SearchQuery,
};
use sillage_test_support::search_budget;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn chunk(id: u64, text: &str) -> IndexedChunk {
    IndexedChunk {
        artifact_id: ArtifactId::new(1),
        chunk_id: ChunkId::new(id),
        text: text.to_string(),
    }
}

fn query(text: &str) -> Result<SearchQuery, Box<dyn std::error::Error>> {
    Ok(SearchQuery {
        q: text.to_string(),
        offset: 0,
        limit: 10,
        execution_budget: search_budget(10)?,
    })
}

#[test]
fn reservation_refusal_preserves_ownership_and_exact_boundary() -> TestResult {
    let budget = Arc::new(RamMemoryBudget::new(10));
    let mut retained = budget.reserve(8)?;
    assert_eq!(budget.used_bytes(), 8);
    assert!(matches!(
        retained.resize(11),
        Err(MemoryBudgetError::Exhausted { .. })
    ));
    assert_eq!(retained.reserved_bytes(), 8);
    assert_eq!(budget.available_bytes(), 2);
    retained.resize(3)?;
    let boundary = budget.reserve(7)?;
    assert_eq!(budget.available_bytes(), 0);
    assert!(matches!(
        budget.reserve(1),
        Err(MemoryBudgetError::Exhausted { .. })
    ));
    drop(retained);
    assert_eq!(budget.available_bytes(), 3);
    drop(boundary);
    assert_eq!(budget.used_bytes(), 0);
    Ok(())
}

#[test]
fn reservation_transfers_and_drops_only_owned_charges() -> TestResult {
    let budget = Arc::new(RamMemoryBudget::new(10));
    let mut retained = budget.reserve(8)?;
    let output_credit = retained
        .split_off(3)
        .ok_or_else(|| std::io::Error::other("reservation split was refused"))?;
    assert_eq!(retained.reserved_bytes(), 5);
    assert_eq!(budget.used_bytes(), 8);
    drop(output_credit);
    assert_eq!(budget.used_bytes(), 5);

    let output_credit = budget.reserve(4)?;
    assert!(retained.can_merge(&output_credit));
    if let Err(output_credit) = retained.merge(output_credit) {
        drop(output_credit);
        return Err(std::io::Error::other("same-budget reservation merge failed").into());
    }
    assert_eq!(retained.reserved_bytes(), 9);
    assert_eq!(budget.used_bytes(), 9);
    assert!(retained.split_off(10).is_none());
    assert_eq!(retained.reserved_bytes(), 9);

    let foreign_budget = Arc::new(RamMemoryBudget::new(1));
    let foreign = foreign_budget.reserve(1)?;
    assert!(!retained.can_merge(&foreign));
    if let Err(foreign) = retained.merge(foreign) {
        assert_eq!(foreign.reserved_bytes(), 1);
        drop(foreign);
    } else {
        return Err(std::io::Error::other("cross-budget reservation merge succeeded").into());
    }
    assert_eq!(budget.used_bytes(), 9);
    assert_eq!(foreign_budget.used_bytes(), 0);
    drop(retained);
    assert_eq!(budget.used_bytes(), 0);
    Ok(())
}

#[test]
fn constructor_refuses_writer_estimate_without_retained_index_state() -> TestResult {
    let budget = Arc::new(RamMemoryBudget::new(14_999_999));
    let result = TantivyFullTextIndex::in_memory_with_budget(Arc::clone(&budget), 15_000_000);
    assert!(matches!(
        result,
        Err(InMemoryIndexError::MemoryBudget(
            MemoryBudgetError::Exhausted {
                requested: 15_000_000,
                ..
            }
        ))
    ));
    assert_eq!(budget.used_bytes(), 0);
    Ok(())
}

#[test]
fn mixed_batch_quota_refusal_preserves_replacements_and_pending_documents() -> TestResult {
    let budget = Arc::new(RamMemoryBudget::new(67_108_864));
    let index = TantivyFullTextIndex::in_memory_with_budget(Arc::clone(&budget), 15_000_000)?;
    index.index_chunks(vec![chunk(1, "original"), chunk(2, "pending")])?;
    let retained = budget.reserve(budget.available_bytes().saturating_sub(8192))?;
    let result = index.index_artifact_chunks(
        vec![chunk(1, "replacement")],
        vec![IndexedCard {
            artifact_id: ArtifactId::new(1),
            card_id: CardId::new(1),
            title: "replacement".to_string(),
            body: "growth ".repeat(64),
        }],
        vec![IndexedLexicalChunk {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(3),
            text: "replacement".to_string(),
            path: Some("scope/file".to_string()),
            filename: Some("file".to_string()),
            symbol: None,
        }],
        vec![IndexedLexicalCard {
            artifact_id: ArtifactId::new(1),
            card_id: CardId::new(2),
            title: "replacement".to_string(),
            body: "growth ".repeat(64),
            path: None,
            filename: None,
            symbol: None,
        }],
    );
    assert!(result.is_err());
    assert!(matches!(
        index.take_memory_error(),
        Some(MemoryBudgetError::Exhausted { .. })
    ));
    drop(retained);
    index.commit_and_reload()?;
    assert_eq!(index.doc_count()?, 2);
    let original = index.search(query("original")?)?;
    assert_eq!(original.hits.len(), 1);
    assert_eq!(original.hits[0].chunk.text, "original");
    assert_eq!(index.search(query("pending")?)?.hits.len(), 1);
    assert!(index.search(query("replacement")?)?.hits.is_empty());
    assert!(index.search_cards(query("replacement")?)?.hits.is_empty());
    Ok(())
}

#[test]
fn reload_and_query_refusals_leave_committed_generation_usable() -> TestResult {
    let budget = Arc::new(RamMemoryBudget::new(67_108_864));
    let index = TantivyFullTextIndex::in_memory_with_budget(Arc::clone(&budget), 15_000_000)?;
    index.index_chunks(vec![chunk(1, "original")])?;
    index.commit_and_reload()?;
    index.index_chunks(vec![chunk(2, "second")])?;
    let retained = budget.reserve(budget.available_bytes())?;
    assert!(index.commit_and_reload().is_err());
    assert!(matches!(
        index.take_memory_error(),
        Some(MemoryBudgetError::Exhausted { .. })
    ));
    assert_eq!(index.doc_count()?, 1);
    assert!(index.search(query("original")?).is_err());
    assert!(matches!(
        index.take_memory_error(),
        Some(MemoryBudgetError::Exhausted { .. })
    ));
    drop(retained);
    index.commit_and_reload()?;
    assert_eq!(index.doc_count()?, 2);
    let transferred = index.search(query("original")?)?;
    assert_eq!(transferred.hits.len(), 1);
    drop(index);
    assert_eq!(budget.used_bytes(), 0);
    assert_eq!(transferred.hits[0].chunk.text, "original");
    Ok(())
}

#[test]
fn delete_quota_refusal_preserves_document_until_admitted_retry() -> TestResult {
    let budget = Arc::new(RamMemoryBudget::new(67_108_864));
    let index = TantivyFullTextIndex::in_memory_with_budget(Arc::clone(&budget), 15_000_000)?;
    index.index_chunks(vec![chunk(1, "original")])?;
    index.commit_and_reload()?;
    let retained = budget.reserve(budget.available_bytes())?;
    let keys = [(ArtifactId::new(1), ChunkId::new(1))];
    assert!(index.delete_chunks(&keys).is_err());
    assert!(matches!(
        index.take_memory_error(),
        Some(MemoryBudgetError::Exhausted { .. })
    ));
    drop(retained);
    assert_eq!(index.search(query("original")?)?.hits.len(), 1);
    index.delete_chunks(&keys)?;
    index.commit_and_reload()?;
    assert!(index.search(query("original")?)?.hits.is_empty());
    Ok(())
}
