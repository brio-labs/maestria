//! Tests for issue #421: the degraded vector lane must not starve the
//! full-text/parse lanes.
//!
//! Three observable contracts are defended here:
//! 1. `IndexArtifactVectors` effects admit under a dedicated vector lane, so a vector
//!    flood can neither wait on the main semaphore behind a saturated
//!    full-text lane nor block the main lane from the other side.
//! 2. The vector lane degrades permanently per artifact: the stale-projection
//!    invalidation port runs at most once per artifact instead of once per
//!    chunk.
//! 3. A successfully committed full-text completion never becomes a
//!    retryable failure when the input channel is momentarily full; the
//!    effect succeeds and the completion is delivered once the channel drains.

use crate::config::EffectExecutionContext;
use crate::effect_dispatch::EffectWork;
use crate::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, DomainInput, IndexArtifactVectorsRequest,
    IndexChunkRequest, KernelState, LogicalTick, MaestriaEffect, SourceSpan, StructureNodeId,
};
use maestria_ports::lexical::{IndexedLexicalCard, IndexedLexicalChunk};
use maestria_ports::{
    BoundedSearch, CardHit, FullTextIndex, IndexedCard, IndexedChunk, PortError, SearchHit,
    SearchQuery, VectorIndex, VectorSearchHit, VectorSearchQuery,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

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

fn chunk_fixture(id: ChunkId, artifact_id: ArtifactId, order: u32, text: &str) -> Chunk {
    Chunk {
        id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        representations_digest: "sha256:fixture".to_string(),
        order,
        text: text.into(),
    }
}

/// Vector index whose `delete_chunks` counts every invalidation call and can
/// optionally block inside the call (to saturate the vector lane).
#[derive(Clone)]
struct SpyVectorIndex {
    inner: InMemoryVectorIndex,
    deletes: Arc<AtomicUsize>,
    release_deletes: Arc<AtomicBool>,
}

impl SpyVectorIndex {
    fn new(deletes: Arc<AtomicUsize>, release_deletes: Arc<AtomicBool>) -> Self {
        Self {
            inner: InMemoryVectorIndex::new(),
            deletes,
            release_deletes,
        }
    }
}

impl VectorIndex for SpyVectorIndex {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        self.inner.index_embeddings(embeddings)
    }

    fn search_similar(
        &self,
        query: VectorSearchQuery,
    ) -> Result<BoundedSearch<VectorSearchHit>, PortError> {
        self.inner.search_similar(query)
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        self.deletes.fetch_add(chunk_ids.len(), Ordering::SeqCst);
        while !self.release_deletes.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        self.inner.delete_chunks(chunk_ids)
    }

    fn clear(&self) -> Result<(), PortError> {
        self.inner.clear()
    }
}

/// Full-text index whose `index_artifact_chunk` blocks until released,
/// holding whatever semaphore permit the effect was admitted under.
struct BlockingFullTextIndex {
    inner: InMemoryFullTextIndex,
    entered: Arc<AtomicUsize>,
    release: Arc<AtomicBool>,
}

impl FullTextIndex for BlockingFullTextIndex {
    fn supports_lexical_metadata(&self) -> bool {
        true
    }

    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        self.inner.index_chunks(chunks)
    }

    fn search(&self, query: SearchQuery) -> Result<BoundedSearch<SearchHit>, PortError> {
        self.inner.search(query)
    }

    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError> {
        self.inner.index_cards(cards)
    }

    fn search_cards(&self, query: SearchQuery) -> Result<BoundedSearch<CardHit>, PortError> {
        self.inner.search_cards(query)
    }

    fn index_lexical_chunks(&self, chunks: Vec<IndexedLexicalChunk>) -> Result<(), PortError> {
        self.inner.index_lexical_chunks(chunks)
    }

    fn index_lexical_cards(&self, cards: Vec<IndexedLexicalCard>) -> Result<(), PortError> {
        self.inner.index_lexical_cards(cards)
    }

    fn index_artifact_chunks(
        &self,
        chunks: Vec<IndexedChunk>,
        cards: Vec<IndexedCard>,
        lexical_chunks: Vec<IndexedLexicalChunk>,
        lexical_cards: Vec<IndexedLexicalCard>,
    ) -> Result<(), PortError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        self.inner
            .index_artifact_chunks(chunks, cards, lexical_chunks, lexical_cards)
    }
}

/// Register six degradable vector artifacts (ids 100..106) with one chunk
/// each, so a no-provider artifact effect has stale rows to invalidate.
fn register_degradable_flood_fixtures(state: &mut KernelState) {
    for i in 0..6usize {
        let artifact = ArtifactId::new(100 + i as u64);
        let chunk = ChunkId::new(1000 + i as u64);
        Arc::make_mut(&mut state.artifacts).insert(artifact, artifact_fixture(artifact));
        Arc::make_mut(&mut state.chunks)
            .insert(chunk, chunk_fixture(chunk, artifact, 0, "clean chunk text"));
        Arc::make_mut(&mut state.pending_vector_chunks).insert(chunk);
    }
}

/// A full-text effect on the main lane, plus a degrading vector flood on the
/// vector lane: the flood must admit (and complete) while the main lane is
/// saturated, and the blocked full-text effect must still deliver its
/// completion when released.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vector_effects_admit_while_main_lane_is_saturated()
-> Result<(), Box<dyn std::error::Error>> {
    let release_full_text = Arc::new(AtomicBool::new(false));
    let entered_full_text = Arc::new(AtomicUsize::new(0));
    let search_index = Arc::new(BlockingFullTextIndex {
        inner: InMemoryFullTextIndex::new(),
        entered: entered_full_text.clone(),
        release: release_full_text.clone(),
    });
    let deletes = Arc::new(AtomicUsize::new(0));
    let vector_index = Arc::new(SpyVectorIndex::new(
        deletes.clone(),
        Arc::new(AtomicBool::new(true)),
    ));

    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact_fixture(artifact_id));
    Arc::make_mut(&mut state.chunks).insert(
        chunk_id,
        chunk_fixture(chunk_id, artifact_id, 0, "clean full-text chunk"),
    );
    Arc::make_mut(&mut state.pending_full_text).insert(chunk_id);

    register_degradable_flood_fixtures(&mut state);

    let adapters = Adapters {
        search_index: search_index.clone(),
        vector_index: Some(vector_index),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, mut input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            max_retries: 0,
            default_effect_timeout: Duration::from_secs(30),
            ..RuntimeConfig::default()
        },
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    let (effect_tx, effect_rx) = mpsc::channel(32);
    let effect_shutdown = CancellationToken::new();
    let runtime_shutdown = CancellationToken::new();
    let executor =
        runtime.spawn_effect_executor(effect_rx, effect_shutdown.clone(), runtime_shutdown.clone());

    // The single main-lane permit is taken by a full-text effect that blocks
    // inside its search-index commit.
    effect_tx
        .send(vec![EffectWork::Pending(MaestriaEffect::IndexFullText(
            IndexChunkRequest {
                artifact_id,
                chunk_id,
            },
        ))])
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while entered_full_text.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "full-text effect never blocked on the main lane")?;

    // A degrading vector flood (distinct artifacts, no provider): every first
    // chunk per artifact runs the invalidation port, which is observable.
    let flood = (0..6)
        .map(|i| {
            EffectWork::Pending(MaestriaEffect::IndexArtifactVectors(
                IndexArtifactVectorsRequest::new(ArtifactId::new(100 + i)),
            ))
        })
        .collect::<Vec<_>>();
    effect_tx.send(flood).await?;

    tokio::time::timeout(Duration::from_secs(5), async {
        while deletes.load(Ordering::SeqCst) < 6 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "vector effects must admit on the vector lane while the main lane is saturated")?;
    assert!(
        !release_full_text.load(Ordering::SeqCst),
        "main lane must still be saturated while the vector flood drains"
    );

    // Release the full-text commit: its completion must be delivered even
    // though the vector flood ran concurrently on the other lane.
    release_full_text.store(true, Ordering::SeqCst);
    let completion = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await
        .map_err(|_| "full-text completion never delivered after release")?
        .ok_or("input channel closed")?;
    assert!(
        matches!(completion, DomainInput::FullTextIndexCompleted(completion)
            if completion.artifact_id == artifact_id && completion.chunk_id == chunk_id),
        "the blocked full-text effect must complete its artifact chunk"
    );

    drop(effect_tx);
    tokio::time::timeout(Duration::from_secs(5), executor).await??;
    Ok(())
}

/// The executor must keep consuming batches while the lanes are saturated:
/// effects wait for their permits inside their own task, so a later batch's
/// vector effect still runs even when the single main-lane permit is held by
/// a blocked full-text effect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn executor_consumes_batches_while_lane_is_saturated()
-> Result<(), Box<dyn std::error::Error>> {
    let release_full_text = Arc::new(AtomicBool::new(false));
    let entered_full_text = Arc::new(AtomicUsize::new(0));
    let search_index = Arc::new(BlockingFullTextIndex {
        inner: InMemoryFullTextIndex::new(),
        entered: entered_full_text.clone(),
        release: release_full_text.clone(),
    });
    let deletes = Arc::new(AtomicUsize::new(0));
    let vector_index = Arc::new(SpyVectorIndex::new(
        deletes.clone(),
        Arc::new(AtomicBool::new(true)),
    ));

    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact_fixture(artifact_id));
    Arc::make_mut(&mut state.chunks).insert(
        chunk_id,
        chunk_fixture(chunk_id, artifact_id, 0, "clean full-text chunk"),
    );
    Arc::make_mut(&mut state.pending_full_text).insert(chunk_id);

    register_degradable_flood_fixtures(&mut state);

    let adapters = Adapters {
        search_index: search_index.clone(),
        vector_index: Some(vector_index),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, mut input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            max_retries: 0,
            default_effect_timeout: Duration::from_secs(30),
            ..RuntimeConfig::default()
        },
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    let (effect_tx, effect_rx) = mpsc::channel(32);
    let effect_shutdown = CancellationToken::new();
    let runtime_shutdown = CancellationToken::new();
    let executor =
        runtime.spawn_effect_executor(effect_rx, effect_shutdown.clone(), runtime_shutdown.clone());

    // The only main-lane permit is taken by a blocking full-text effect.
    effect_tx
        .send(vec![EffectWork::Pending(MaestriaEffect::IndexFullText(
            IndexChunkRequest {
                artifact_id,
                chunk_id,
            },
        ))])
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while entered_full_text.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "first full-text effect never blocked on the main lane")?;

    // A second full-text effect (needs the saturated main lane) followed by
    // a vector effect: the executor must consume both batches immediately and
    // let the vector effect run on its own lane.
    effect_tx
        .send(vec![EffectWork::Pending(MaestriaEffect::IndexFullText(
            IndexChunkRequest {
                artifact_id,
                chunk_id,
            },
        ))])
        .await?;
    effect_tx
        .send(vec![EffectWork::Pending(
            MaestriaEffect::IndexArtifactVectors(IndexArtifactVectorsRequest::new(
                ArtifactId::new(101),
            )),
        )])
        .await?;

    tokio::time::timeout(Duration::from_secs(5), async {
        while deletes.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "executor must consume later batches while the main lane is saturated")?;
    assert_eq!(
        entered_full_text.load(Ordering::SeqCst),
        1,
        "the second full-text effect must still be waiting for the main-lane permit"
    );

    release_full_text.store(true, Ordering::SeqCst);
    let completion = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await
        .map_err(|_| "full-text completion never delivered after release")?
        .ok_or("input channel closed")?;
    assert!(
        matches!(completion, DomainInput::FullTextIndexCompleted(completion)
            if completion.artifact_id == artifact_id && completion.chunk_id == chunk_id)
    );

    drop(effect_tx);
    tokio::time::timeout(Duration::from_secs(5), executor).await??;
    Ok(())
}

/// The vector lane stays saturated (two effects blocked inside their
/// stale-projection invalidation) while a full-text effect runs to completion
/// on the main lane: a vector flood must never block full-text effects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_text_effect_completes_while_vector_lane_is_saturated()
-> Result<(), Box<dyn std::error::Error>> {
    let release_deletes = Arc::new(AtomicBool::new(false));
    let deletes = Arc::new(AtomicUsize::new(0));
    let vector_index = Arc::new(SpyVectorIndex::new(
        deletes.clone(),
        release_deletes.clone(),
    ));

    let full_text_artifact = ArtifactId::new(1);
    let full_text_chunk = ChunkId::new(10);
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.artifacts)
        .insert(full_text_artifact, artifact_fixture(full_text_artifact));
    Arc::make_mut(&mut state.chunks).insert(
        full_text_chunk,
        chunk_fixture(
            full_text_chunk,
            full_text_artifact,
            0,
            "clean full-text chunk",
        ),
    );
    Arc::make_mut(&mut state.pending_full_text).insert(full_text_chunk);

    // The degraded vector artifacts own chunk rows that invalidation clears.
    for (artifact, chunk) in [
        (ArtifactId::new(101), ChunkId::new(1001)),
        (ArtifactId::new(102), ChunkId::new(1002)),
    ] {
        Arc::make_mut(&mut state.artifacts).insert(artifact, artifact_fixture(artifact));
        Arc::make_mut(&mut state.chunks)
            .insert(chunk, chunk_fixture(chunk, artifact, 0, "clean chunk text"));
        Arc::make_mut(&mut state.pending_vector_chunks).insert(chunk);
    }

    let adapters = Adapters {
        vector_index: Some(vector_index),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, mut input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            max_retries: 0,
            default_effect_timeout: Duration::from_secs(30),
            ..RuntimeConfig::default()
        },
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    let (effect_tx, effect_rx) = mpsc::channel(32);
    let effect_shutdown = CancellationToken::new();
    let runtime_shutdown = CancellationToken::new();
    let executor =
        runtime.spawn_effect_executor(effect_rx, effect_shutdown.clone(), runtime_shutdown.clone());

    // Saturate the vector lane: two distinct artifacts, each blocking inside
    // its first (and only) stale-projection invalidation.
    effect_tx
        .send(vec![
            EffectWork::Pending(MaestriaEffect::IndexArtifactVectors(
                IndexArtifactVectorsRequest::new(ArtifactId::new(101)),
            )),
            EffectWork::Pending(MaestriaEffect::IndexArtifactVectors(
                IndexArtifactVectorsRequest::new(ArtifactId::new(102)),
            )),
        ])
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while deletes.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "vector lane never saturated")?;

    // The full-text effect must complete on the main lane while the vector
    // lane stays blocked.
    effect_tx
        .send(vec![EffectWork::Pending(MaestriaEffect::IndexFullText(
            IndexChunkRequest {
                artifact_id: full_text_artifact,
                chunk_id: full_text_chunk,
            },
        ))])
        .await?;
    let completion = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await
        .map_err(|_| "full-text completion never delivered while the vector lane is saturated")?
        .ok_or("input channel closed")?;
    assert!(
        matches!(completion, DomainInput::FullTextIndexCompleted(completion)
            if completion.artifact_id == full_text_artifact
                && completion.chunk_id == full_text_chunk),
        "full-text completion must be delivered while the vector lane is saturated"
    );
    assert!(
        !release_deletes.load(Ordering::SeqCst),
        "vector lane must still be saturated when the full-text effect completes"
    );

    release_deletes.store(true, Ordering::SeqCst);
    drop(effect_tx);
    tokio::time::timeout(Duration::from_secs(5), executor).await??;
    Ok(())
}

/// The stale-projection invalidation port runs at most once per artifact:
/// three chunks of artifact A trigger one invalidation, artifact B gets its
/// own first invalidation.
#[tokio::test]
async fn vector_degradation_invalidates_at_most_once_per_artifact()
-> Result<(), Box<dyn std::error::Error>> {
    let deletes = Arc::new(AtomicUsize::new(0));
    let vector_index = Arc::new(SpyVectorIndex::new(
        deletes.clone(),
        Arc::new(AtomicBool::new(true)),
    ));
    let adapters = Adapters {
        vector_index: Some(vector_index),
        ..crate::test_helpers::test_adapters()
    };
    let artifact_a = ArtifactId::new(1);
    let artifact_b = ArtifactId::new(2);
    let mut state = KernelState::new();
    for (artifact, chunk) in [
        (artifact_a, ChunkId::new(10)),
        (artifact_a, ChunkId::new(11)),
        (artifact_a, ChunkId::new(12)),
        (artifact_b, ChunkId::new(20)),
    ] {
        Arc::make_mut(&mut state.artifacts).insert(artifact, artifact_fixture(artifact));
        Arc::make_mut(&mut state.chunks)
            .insert(chunk, chunk_fixture(chunk, artifact, 0, "clean chunk text"));
        Arc::make_mut(&mut state.pending_vector_chunks).insert(chunk);
    }
    let (input_tx, _input_rx) = mpsc::channel(8);
    let context = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(crate::test_helpers::test_governance()),
        Arc::new(RwLock::new(state)),
        input_tx,
    );

    for artifact_id in [artifact_a, artifact_a, artifact_a, artifact_b] {
        let result = MaestriaRuntime::test_execute_effect(
            MaestriaEffect::IndexArtifactVectors(IndexArtifactVectorsRequest::new(artifact_id)),
            context.clone(),
            None,
        )
        .await;
        assert!(
            !result,
            "vector effect without a provider must degrade, not complete"
        );
    }
    // The spy counts deleted chunk ids: artifact A contributes its 3 chunks,
    // artifact B its 1 — four effects degrade into exactly two invalidation
    // passes (once per artifact), covering 4 ids, not 10.
    assert_eq!(
        deletes.load(Ordering::SeqCst),
        4,
        "stale-projection invalidation must run at most once per artifact"
    );
    Ok(())
}

/// A successfully committed full-text chunk on a full input channel must not
/// become a retryable failure: the effect succeeds (no retry) and the
/// completion is delivered once the channel drains.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_text_completion_on_full_input_channel_delivers_without_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact_fixture(artifact_id));
    Arc::make_mut(&mut state.chunks).insert(
        chunk_id,
        chunk_fixture(chunk_id, artifact_id, 0, "clean full-text chunk"),
    );
    Arc::make_mut(&mut state.pending_full_text).insert(chunk_id);

    let adapters = Adapters {
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        ..crate::test_helpers::test_adapters()
    };
    let (input_tx, mut input_rx) = mpsc::channel(1);
    // Fill the input channel so the completion send cannot land immediately.
    input_tx
        .try_send(DomainInput::ClockTick(LogicalTick::new(0)))
        .map_err(|_| "test input channel should accept one item")?;
    let context = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(crate::test_helpers::test_governance()),
        Arc::new(RwLock::new(state)),
        input_tx,
    );

    // Any retryable failure would surface as Err here; the effect must
    // succeed on the first attempt and defer the completion instead.
    let result = context
        .clone()
        .execute_with_retries(MaestriaEffect::IndexFullText(IndexChunkRequest {
            artifact_id,
            chunk_id,
        }))
        .await;
    assert!(
        result.is_ok(),
        "full-text effect must succeed even when the input channel is full"
    );

    // Drain the channel: the dummy item first, then the deferred completion.
    let first = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await?
        .ok_or("input channel closed before completion")?;
    assert!(
        matches!(first, DomainInput::ClockTick(_)),
        "first drained item must be the filler input"
    );
    let completion = tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
        .await?
        .ok_or("input channel closed before completion")?;
    assert!(
        matches!(completion, DomainInput::FullTextIndexCompleted(completion)
            if completion.artifact_id == artifact_id && completion.chunk_id == chunk_id),
        "the deferred full-text completion must arrive once the channel drains"
    );
    Ok(())
}

#[derive(Clone, Default)]
struct CountingEmbeddingProvider {
    calls: Arc<AtomicUsize>,
}

impl CountingEmbeddingProvider {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl maestria_ports::EmbeddingProvider for CountingEmbeddingProvider {
    fn disclosure(&self) -> maestria_ports::ProviderDisclosure {
        maestria_ports::ProviderDisclosure {
            remote: false,
            retention: maestria_ports::RetentionPolicy::NoRetention,
        }
    }
    fn embed(
        &self,
        request: maestria_ports::EmbeddingRequest,
    ) -> Result<maestria_ports::EmbeddingResponse, PortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(maestria_ports::EmbeddingResponse {
            vector: vec![1.0, 0.0],
            provider_id: "counting".to_string(),
            model: request.model,
            model_version: "v1".to_string(),
            identity: request.identity,
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        })
    }
    fn identity(&self) -> Option<maestria_ports::EmbeddingIdentity> {
        maestria_ports::contract_tests::fixture_embedding_identity("counting", 2).ok()
    }
}

/// Secret-bearing chunks must degrade the vector lane instead of failing the
/// runtime: a later clean chunk is still embedded, proving the runtime did
/// not cancel after the secret refusal.
#[tokio::test]
async fn secret_chunk_degrades_vector_lane_without_cancelling_runtime()
-> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(1);
    let secret_chunk = ChunkId::new(10);
    let clean_artifact = ArtifactId::new(2);
    let clean_chunk = ChunkId::new(11);
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact_fixture(artifact_id));
    Arc::make_mut(&mut state.artifacts).insert(clean_artifact, artifact_fixture(clean_artifact));
    Arc::make_mut(&mut state.chunks).insert(
        secret_chunk,
        chunk_fixture(secret_chunk, artifact_id, 0, "api_key = abc123"),
    );
    Arc::make_mut(&mut state.chunks).insert(
        clean_chunk,
        chunk_fixture(clean_chunk, clean_artifact, 0, "clean chunk text"),
    );
    Arc::make_mut(&mut state.pending_vector_chunks).insert(secret_chunk);
    Arc::make_mut(&mut state.pending_vector_chunks).insert(clean_chunk);

    let provider = CountingEmbeddingProvider::default();
    let adapters = Adapters {
        embedding_provider: Some(Arc::new(provider.clone())),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, mut _input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            max_retries: 0,
            default_effect_timeout: Duration::from_secs(30),
            embedding_model: Some("counting".to_string()),
            ..RuntimeConfig::default()
        },
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    let (effect_tx, effect_rx) = mpsc::channel(32);
    let effect_shutdown = CancellationToken::new();
    let runtime_shutdown = CancellationToken::new();
    let executor =
        runtime.spawn_effect_executor(effect_rx, effect_shutdown.clone(), runtime_shutdown.clone());

    effect_tx
        .send(vec![EffectWork::Pending(
            MaestriaEffect::IndexArtifactVectors(IndexArtifactVectorsRequest::new(artifact_id)),
        )])
        .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        provider.calls(),
        0,
        "secret-bearing chunk must not reach the embedding provider"
    );

    effect_tx
        .send(vec![EffectWork::Pending(
            MaestriaEffect::IndexArtifactVectors(IndexArtifactVectorsRequest::new(clean_artifact)),
        )])
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while provider.calls() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "clean vector effect never ran: runtime cancelled after secret refusal")?;

    drop(effect_tx);
    tokio::time::timeout(Duration::from_secs(5), executor).await??;
    Ok(())
}
