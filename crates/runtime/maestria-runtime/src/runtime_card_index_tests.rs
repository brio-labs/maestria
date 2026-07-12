use super::*;
use maestria_domain::{ArtifactId, Card, CardId, Chunk, ChunkId, KernelState, MaestriaEffect};
use maestria_governance::{DefaultApprovalGate, DefaultRiskClassifier};
use maestria_ports::{
    FullTextIndex, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository, InMemoryFullTextIndex,
    InMemoryGraphIndex, InMemoryHarnessAdapter, InMemoryParser, InMemoryVectorIndex,
    InMemoryWebFetcher, SearchQuery,
};
use std::collections::BTreeSet;
use std::sync::Arc;

#[tokio::test]
async fn index_full_text_effect_indexes_cards_before_chunks() {
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let card_id = CardId::new(100);

    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        order: 0,
        text: "chunk text for indexing".into(),
    };
    let card = Card {
        id: card_id,
        artifact_id,
        title: "indexed card title".into(),
        body: "indexed card body".into(),
        claim_ids: BTreeSet::new(),
    };

    let mut state = KernelState::new();
    state.chunks.insert(chunk_id, chunk);
    state.cards.insert(card_id, card);

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);

    let adapters = Arc::new(Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: search_index.clone(),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    });
    let governance = Arc::new(Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    });

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id,
            chunk_id,
        }),
        adapters.clone(),
        governance,
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(state)),
        input_tx,
        None,
    )
    .await;
    assert!(result, "IndexFullText effect should succeed");

    // Cards should be indexed and searchable.
    let card_hits = search_index
        .search_cards(SearchQuery {
            q: "indexed card".into(),
            limit: 10,
        })
        .expect("search_cards should succeed");
    assert_eq!(card_hits.len(), 1, "one card should match");
    assert_eq!(card_hits[0].card.artifact_id, artifact_id);
    assert_eq!(card_hits[0].card.card_id, card_id);
    assert_eq!(card_hits[0].card.title, "indexed card title");

    // Chunks should also be indexed.
    let chunk_hits = search_index
        .search(SearchQuery {
            q: "chunk text".into(),
            limit: 10,
        })
        .expect("search should succeed");
    assert_eq!(chunk_hits.len(), 1, "one chunk should match");
}

#[tokio::test]
async fn index_full_text_effect_no_cards_when_state_has_none() {
    let artifact_id = ArtifactId::new(2);
    let chunk_id = ChunkId::new(20);

    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        order: 0,
        text: "chunk without cards".into(),
    };

    let mut state = KernelState::new();
    state.chunks.insert(chunk_id, chunk);
    // No cards inserted.

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);

    let adapters = Arc::new(Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: search_index.clone(),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    });
    let governance = Arc::new(Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    });

    let result = MaestriaRuntime::execute_effect(
        MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id,
            chunk_id,
        }),
        adapters.clone(),
        governance,
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(state)),
        input_tx,
        None,
    )
    .await;
    assert!(result, "IndexFullText effect should succeed without cards");

    // No cards indexed when state had none.
    let card_hits = search_index
        .search_cards(SearchQuery {
            q: "anything".into(),
            limit: 10,
        })
        .expect("search_cards should succeed");
    assert!(
        card_hits.is_empty(),
        "no cards should be indexed when state lacks cards"
    );

    // Chunks still indexed.
    let chunk_hits = search_index
        .search(SearchQuery {
            q: "chunk without cards".into(),
            limit: 10,
        })
        .expect("search should succeed");
    assert_eq!(chunk_hits.len(), 1, "chunk should still be indexed");
}

#[tokio::test]
async fn index_full_text_effect_reindexing_is_idempotent() {
    let artifact_id = ArtifactId::new(3);
    let chunk_id = ChunkId::new(30);
    let card_id = CardId::new(300);

    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        order: 0,
        text: "reindexed chunk".into(),
    };
    let card = Card {
        id: card_id,
        artifact_id,
        title: "reindexed card".into(),
        body: "reindexed body".into(),
        claim_ids: BTreeSet::new(),
    };

    let mut state = KernelState::new();
    state.chunks.insert(chunk_id, chunk);
    state.cards.insert(card_id, card);

    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let (input_tx, _input_rx) = mpsc::channel(8);

    let adapters = Arc::new(Adapters {
        event_log: Arc::new(InMemoryEventLog::new()),
        blob_store: Arc::new(InMemoryBlobStore::new()),
        search_index: search_index.clone(),
        harness: Arc::new(InMemoryHarnessAdapter::new()),
        parser: Arc::new(InMemoryParser::new()),
        artifact_repo: Arc::new(InMemoryArtifactRepository::new()),
        chunk_repo: Arc::new(InMemoryChunkRepository::new()),
        card_repo: Arc::new(InMemoryCardRepository::new()),
        evidence_repo: Arc::new(InMemoryEvidenceRepository::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        graph_index: Arc::new(InMemoryGraphIndex::new()),
        web_fetcher: Arc::new(InMemoryWebFetcher::new()),
    });
    let governance = Arc::new(Governance {
        classifier: Arc::new(DefaultRiskClassifier),
        approval_gate: Arc::new(DefaultApprovalGate),
    });

    let effect = MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
        artifact_id,
        chunk_id,
    });

    // First indexing.
    let result = MaestriaRuntime::execute_effect(
        effect.clone(),
        adapters.clone(),
        governance.clone(),
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(state.clone())),
        input_tx.clone(),
        None,
    )
    .await;
    assert!(result, "first IndexFullText should succeed");

    // Second indexing (recovery / re-drive).
    let result = MaestriaRuntime::execute_effect(
        effect,
        adapters.clone(),
        governance,
        AutonomyProfile::TrustedWorkspace,
        Scope::default(),
        Arc::new(RwLock::new(state)),
        input_tx,
        None,
    )
    .await;
    assert!(result, "second IndexFullText should succeed");

    // Still exactly one card in the index.
    let card_hits = search_index
        .search_cards(SearchQuery {
            q: "reindexed".into(),
            limit: 10,
        })
        .expect("search_cards should succeed");
    assert_eq!(card_hits.len(), 1, "reindexing must not duplicate cards");
    assert_eq!(card_hits[0].card.card_id, card_id);
}
