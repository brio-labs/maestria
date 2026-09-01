//! Shutdown drain: with `drain_effects_on_shutdown`, the
//! runtime keeps servicing domain inputs while in-flight effects finish, so
//! an effect that completes after the shutdown token is cancelled still
//! delivers (and persists) its completion input instead of racing a closed
//! channel. Quiet sessions must still exit promptly via the executor's
//! quiescence signal.

use crate::test_helpers::test_adapters;
use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, DomainEvent, KernelState, StartFullTextIndex,
};
use maestria_ports::{
    BoundedSearch, CardHit, EventFilter, EventLog, FullTextIndex, InMemoryEventLog,
    InMemoryFullTextIndex, IndexedCard, IndexedChunk, IndexedLexicalCard, IndexedLexicalChunk,
    PortError, SearchHit, SearchQuery,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn artifact_fixture(id: ArtifactId) -> Artifact {
    Artifact {
        id,
        title: "drain artifact".into(),
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
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        representations_digest: "sha256:fixture".to_string(),
        order,
        text: text.into(),
    }
}

/// Full-text index whose artifact commit takes a deterministic wall-clock
/// delay, so the shutdown token can be cancelled while the effect runs.
struct DelayedFullTextIndex {
    inner: InMemoryFullTextIndex,
    delay: Duration,
}

impl FullTextIndex for DelayedFullTextIndex {
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
        std::thread::sleep(self.delay);
        self.inner
            .index_artifact_chunks(chunks, cards, lexical_chunks, lexical_cards)
    }

    fn delete_chunks(
        &self,
        chunk_ids: &[(maestria_domain::ArtifactId, maestria_domain::ChunkId)],
    ) -> Result<(), PortError> {
        self.inner.delete_chunks(chunk_ids)
    }

    fn clear(&self) -> Result<(), PortError> {
        self.inner.clear()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_completing_after_cancellation_persists_via_drain()
-> Result<(), Box<dyn std::error::Error>> {
    let event_log = Arc::new(InMemoryEventLog::new());
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let mut state = KernelState::new();
    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact_fixture(artifact_id));
    Arc::make_mut(&mut state.chunks).insert(
        chunk_id,
        chunk_fixture(chunk_id, artifact_id, 0, "drain test chunk"),
    );
    Arc::make_mut(&mut state.pending_full_text).insert(chunk_id);

    let adapters = crate::Adapters {
        event_log: event_log.clone(),
        search_index: Arc::new(DelayedFullTextIndex {
            inner: InMemoryFullTextIndex::new(),
            delay: Duration::from_millis(200),
        }),
        ..test_adapters()
    };
    let (mut runtime, input_rx) = crate::MaestriaRuntime::new(
        crate::RuntimeConfig::default(),
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    runtime.config.shutdown_drain_grace = Duration::from_secs(2);
    let runtime = runtime.with_graceful_shutdown();

    let handle = runtime.handle();
    let shutdown_token = CancellationToken::new();
    let run_handle = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    // The reply returns once the batch is admitted; the effect is still
    // running inside the delayed port at this point.
    handle
        .submit_durable(maestria_domain::DomainInput::StartFullTextIndex(
            StartFullTextIndex { artifact_id },
        ))
        .await?;
    shutdown_token.cancel();

    // Quiescence, not the two-second grace, must end the drain: the
    // session is quiet apart from the one short effect. A grace-expiry
    // shutdown would take the full two seconds and trip this bound.
    let joined = tokio::time::timeout(Duration::from_millis(1500), run_handle)
        .await
        .map_err(|_| "shutdown waited out the drain grace instead of exiting on quiescence")?;
    let run_result = joined.map_err(|error| format!("runtime join failed: {error}"))?;
    run_result.map_err(|error| format!("runtime run failed: {error}"))?;

    // The completion delivered through the drain and its event persisted.
    let events = event_log.scan(EventFilter { artifact_id: None })?;
    assert!(
        events
            .iter()
            .any(|envelope| matches!(envelope.event, DomainEvent::FullTextIndexed { .. })),
        "the mid-shutdown effect completion must be persisted by the drain; events: {events:?}"
    );
    Ok(())
}
