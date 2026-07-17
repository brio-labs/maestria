use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, KernelState, MaestriaEffect, SourceSpan,
    StructureNodeId,
};
use maestria_ports::{FullTextIndex, InMemoryFullTextIndex, SearchQuery};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

fn artifact_fixture(id: ArtifactId) -> Artifact {
    Artifact {
        id,
        title: "artifact".into(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: Default::default(),
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    }
}

#[tokio::test]
async fn index_full_text_effect_indexes_cards_before_chunks()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let card_id = CardId::new(100);

    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "chunk text for indexing".into(),
    };
    let card = Card {
        id: card_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        title: "indexed card title".into(),
        body: "indexed card body".into(),
        claim_ids: BTreeSet::new(),
        security: maestria_domain::SecurityMetadata::default(),
    };

    let mut state = KernelState::new();
    state
        .artifacts
        .insert(artifact_id, artifact_fixture(artifact_id));
    state.chunks.insert(chunk_id, chunk);
    state.cards.insert(card_id, card);

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);

    let adapters = Arc::new(Adapters {
        search_index: search_index.clone(),
        ..crate::test_helpers::test_adapters()
    });
    let governance = Arc::new(crate::test_helpers::test_governance());

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id,
            chunk_id,
        }),
        EffectExecutionContext::test_default(
            adapters.clone(),
            governance,
            Arc::new(RwLock::new(state)),
            input_tx,
        ),
        None,
    )
    .await;
    assert!(result, "IndexFullText effect should succeed");

    // Cards should be indexed and searchable.
    let card_hits = search_index.search_cards(SearchQuery {
        q: "indexed card".into(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(card_hits.len(), 1, "one card should match");
    assert_eq!(card_hits[0].card.artifact_id, artifact_id);
    assert_eq!(card_hits[0].card.card_id, card_id);
    assert_eq!(card_hits[0].card.title, "indexed card title");

    // Chunks should also be indexed.
    let chunk_hits = search_index.search(SearchQuery {
        q: "chunk text".into(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(chunk_hits.len(), 1, "one chunk should match");
    Ok(())
}

#[tokio::test]
async fn index_full_text_effect_no_cards_when_state_has_none()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(2);
    let chunk_id = ChunkId::new(20);

    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "chunk without cards".into(),
    };

    let mut state = KernelState::new();
    state
        .artifacts
        .insert(artifact_id, artifact_fixture(artifact_id));
    state.chunks.insert(chunk_id, chunk);
    // No cards inserted.

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);

    let adapters = Arc::new(Adapters {
        search_index: search_index.clone(),
        ..crate::test_helpers::test_adapters()
    });
    let governance = Arc::new(crate::test_helpers::test_governance());

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id,
            chunk_id,
        }),
        EffectExecutionContext::test_default(
            adapters.clone(),
            governance,
            Arc::new(RwLock::new(state)),
            input_tx,
        ),
        None,
    )
    .await;
    assert!(result, "IndexFullText effect should succeed without cards");

    // No cards indexed when state had none.
    let card_hits = search_index.search_cards(SearchQuery {
        q: "anything".into(),
        limit: 10,
        offset: 0,
    })?;
    assert!(
        card_hits.is_empty(),
        "no cards should be indexed when state lacks cards"
    );

    // Chunks still indexed.
    let chunk_hits = search_index.search(SearchQuery {
        q: "chunk without cards".into(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(chunk_hits.len(), 1, "chunk should still be indexed");
    Ok(())
}

#[tokio::test]
async fn index_full_text_effect_reindexing_is_idempotent() -> Result<(), Box<dyn std::error::Error>>
{
    let artifact_id = ArtifactId::new(3);
    let chunk_id = ChunkId::new(30);
    let card_id = CardId::new(300);

    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "reindexed chunk".into(),
    };
    let card = Card {
        id: card_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        title: "reindexed card".into(),
        body: "reindexed body".into(),
        claim_ids: BTreeSet::new(),
        security: maestria_domain::SecurityMetadata::default(),
    };

    let mut state = KernelState::new();
    state
        .artifacts
        .insert(artifact_id, artifact_fixture(artifact_id));
    state.chunks.insert(chunk_id, chunk);
    state.cards.insert(card_id, card);

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);

    let adapters = Arc::new(Adapters {
        search_index: search_index.clone(),
        ..crate::test_helpers::test_adapters()
    });
    let governance = Arc::new(crate::test_helpers::test_governance());

    let effect = MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
        artifact_id,
        chunk_id,
    });

    // First indexing.
    let result = MaestriaRuntime::test_execute_effect(
        effect.clone(),
        EffectExecutionContext::test_default(
            adapters.clone(),
            governance.clone(),
            Arc::new(RwLock::new(state.clone())),
            input_tx.clone(),
        ),
        None,
    )
    .await;
    assert!(result, "first IndexFullText should succeed");

    // Second indexing (recovery / re-drive).
    let result = MaestriaRuntime::test_execute_effect(
        effect,
        EffectExecutionContext::test_default(
            adapters.clone(),
            governance,
            Arc::new(RwLock::new(state)),
            input_tx,
        ),
        None,
    )
    .await;
    assert!(result, "second IndexFullText should succeed");

    // Still exactly one card in the index.
    let card_hits = search_index.search_cards(SearchQuery {
        q: "reindexed".into(),
        limit: 10,
        offset: 0,
    })?;
    assert_eq!(card_hits.len(), 1, "reindexing must not duplicate cards");
    assert_eq!(card_hits[0].card.card_id, card_id);
    Ok(())
}

#[tokio::test]
async fn index_full_text_rejects_secret_bearing_chunk() -> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(9);
    let chunk_id = ChunkId::new(90);
    let mut state = KernelState::new();
    state.chunks.insert(
        chunk_id,
        Chunk {
            id: chunk_id,
            artifact_id,
            node_id: StructureNodeId::new(0),
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: vec![],
            order: 0,
            text: "password=do-not-index".into(),
        },
    );

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id,
            chunk_id,
        }),
        EffectExecutionContext::test_default(
            Arc::new(Adapters {
                search_index: search_index.clone(),
                ..crate::test_helpers::test_adapters()
            }),
            Arc::new(crate::test_helpers::test_governance()),
            Arc::new(RwLock::new(state)),
            input_tx,
        ),
        None,
    )
    .await;

    assert!(!result);
    assert!(
        search_index
            .search(SearchQuery {
                q: "do-not-index".into(),
                limit: 10,
                offset: 0,
            })?
            .is_empty()
    );
    Ok(())
}
