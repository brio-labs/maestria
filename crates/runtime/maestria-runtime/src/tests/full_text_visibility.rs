use maestria_domain::{
    Artifact, ArtifactId, BlobId, Chunk, ChunkId, ContentHash, DomainInput, Evidence, EvidenceKind,
    IndexChunkRequest, IndexStatus, LineRange, LogicalTick, ParseStatus, SnapshotRef, SourceSpan,
    StructureNodeId, evidence_id_for,
};
use maestria_ports::{
    BoundedSearch, CardHit, FullTextIndex, IndexedCard, IndexedChunk, IndexedLexicalCard,
    IndexedLexicalChunk, PortError, SearchHit, SearchQuery,
};
use maestria_search_tantivy::TantivyFullTextIndex;
use parking_lot::{Condvar, Mutex};
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{RwLock, mpsc, oneshot};

struct CommitGate {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release_state: (Mutex<bool>, Condvar),
    commits: AtomicUsize,
}

impl CommitGate {
    fn pause(&self) {
        if let Some(entered) = self.entered.lock().take() {
            let _ = entered.send(());
        }
        let (released, wake) = &self.release_state;
        let mut released = released.lock();
        while !*released {
            wake.wait(&mut released);
        }
    }

    fn release(&self) {
        let (released, wake) = &self.release_state;
        *released.lock() = true;
        wake.notify_all();
    }
}

struct ReleaseCommitOnDrop(Arc<CommitGate>);

impl Drop for ReleaseCommitOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct PausedCommitIndex {
    inner: Arc<TantivyFullTextIndex>,
    gate: Arc<CommitGate>,
}

impl FullTextIndex for PausedCommitIndex {
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

    fn commit_and_reload(&self) -> Result<(), PortError> {
        self.gate.commits.fetch_add(1, Ordering::SeqCst);
        self.gate.pause();
        self.inner.commit_and_reload()
    }

    fn delete_chunks(&self, chunks: &[(ArtifactId, ChunkId)]) -> Result<(), PortError> {
        self.inner.delete_chunks(chunks)
    }

    fn clear(&self) -> Result<(), PortError> {
        self.inner.clear()
    }

    fn search_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(ChunkId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<SearchHit>, PortError> {
        self.inner.search_filtered(query, filter)
    }

    fn search_cards_filtered(
        &self,
        query: SearchQuery,
        filter: &dyn Fn(maestria_domain::CardId, ArtifactId) -> Result<bool, PortError>,
    ) -> Result<BoundedSearch<CardHit>, PortError> {
        self.inner.search_cards_filtered(query, filter)
    }

    fn supports_lexical_metadata(&self) -> bool {
        self.inner.supports_lexical_metadata()
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
        self.inner
            .index_artifact_chunks(chunks, cards, lexical_chunks, lexical_cards)
    }
}

fn add_pending_artifact_chunk(
    state: &mut maestria_domain::KernelState,
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    text: &str,
    hash_seed: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let hash = ContentHash::new(format!("sha256:{hash_seed:064x}"))?;
    let evidence_id = evidence_id_for(artifact_id, 0);
    let artifact = Artifact {
        id: artifact_id,
        title: format!("artifact-{hash_seed}"),
        chunk_ids: BTreeSet::from([chunk_id]),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::from([evidence_id]),
        index_status: IndexStatus::Pending,
        content_hash: Some(hash.clone()),
        parse_status: Some(ParseStatus::Parsed),
        security: Default::default(),
    };
    let chunk = Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(hash_seed),
        source_span: SourceSpan::text_span(1, 1)?,
        representations: Vec::new(),
        representations_digest: format!("sha256:{}", "0".repeat(64)),
        order: 0,
        text: text.to_string(),
    };
    let evidence = Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: format!("/workspace/artifact-{hash_seed}.txt"),
            range: LineRange::new(1, 1)?,
            snapshot: SnapshotRef::new(BlobId::new(42), hash),
        },
        excerpt: text.to_string(),
        observed_at: LogicalTick::new(hash_seed),
        security: Default::default(),
    };

    Arc::make_mut(&mut state.artifacts).insert(artifact_id, artifact);
    Arc::make_mut(&mut state.chunks).insert(chunk_id, chunk);
    Arc::make_mut(&mut state.evidences).insert(evidence_id, evidence);
    Arc::make_mut(&mut state.pending_full_text).insert(chunk_id);
    Ok(())
}

fn search_hits(
    index: &TantivyFullTextIndex,
    query: &str,
) -> Result<Vec<SearchHit>, Box<dyn std::error::Error>> {
    Ok(index
        .search(SearchQuery {
            q: query.to_string(),
            limit: 10,
            offset: 0,
            execution_budget: maestria_test_support::search_budget(10)?,
        })?
        .hits)
}

fn seed_prior_and_pending_index_state(
    index: &TantivyFullTextIndex,
) -> Result<maestria_domain::KernelState, Box<dyn std::error::Error>> {
    let edited_artifact = ArtifactId::new(1);
    let deleted_artifact = ArtifactId::new(2);
    let added_artifact = ArtifactId::new(3);
    let edited_chunk = ChunkId::new(101);
    let deleted_chunk = ChunkId::new(102);
    let added_chunk = ChunkId::new(103);
    index.index_chunks(vec![
        IndexedChunk {
            artifact_id: edited_artifact,
            chunk_id: edited_chunk,
            text: "previouseditterm".to_string(),
        },
        IndexedChunk {
            artifact_id: deleted_artifact,
            chunk_id: deleted_chunk,
            text: "removedfromsearchterm".to_string(),
        },
    ])?;
    index.commit_and_reload()?;

    let mut state = maestria_domain::KernelState::new();
    add_pending_artifact_chunk(
        &mut state,
        edited_artifact,
        edited_chunk,
        "currenteditterm",
        1,
    )?;
    add_pending_artifact_chunk(
        &mut state,
        added_artifact,
        added_chunk,
        "newartifactterm",
        3,
    )?;
    Ok(state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_text_completion_waits_for_searchable_edit_and_delete_across_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let index = Arc::new(TantivyFullTextIndex::open(directory.path())?);
    let edited_artifact = ArtifactId::new(1);
    let deleted_artifact = ArtifactId::new(2);
    let added_artifact = ArtifactId::new(3);
    let edited_chunk = ChunkId::new(101);
    let deleted_chunk = ChunkId::new(102);
    let added_chunk = ChunkId::new(103);

    let state = seed_prior_and_pending_index_state(&index)?;

    // The deletion is buffered with the two ingestion updates and must not be
    // made visible early by an ordinary search.
    index.delete_chunks(&[(deleted_artifact, deleted_chunk)])?;
    assert_eq!(search_hits(&index, "previouseditterm")?.len(), 1);
    assert_eq!(search_hits(&index, "removedfromsearchterm")?.len(), 1);
    assert!(search_hits(&index, "currenteditterm")?.is_empty());
    assert!(search_hits(&index, "newartifactterm")?.is_empty());

    let (entered_tx, entered_rx) = oneshot::channel();
    let gate = Arc::new(CommitGate {
        entered: Mutex::new(Some(entered_tx)),
        release_state: (Mutex::new(false), Condvar::new()),
        commits: AtomicUsize::new(0),
    });
    let _release_on_drop = ReleaseCommitOnDrop(Arc::clone(&gate));
    let adapters = Arc::new(crate::Adapters {
        search_index: Arc::new(PausedCommitIndex {
            inner: Arc::clone(&index),
            gate: Arc::clone(&gate),
        }),
        ..crate::test_helpers::test_adapters()
    });
    let governance = Arc::new(crate::test_helpers::test_governance());
    let state = Arc::new(RwLock::new(state));
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let context = crate::EffectExecutionContext::test_default(
        adapters.clone(),
        governance,
        Arc::clone(&state),
        input_tx,
    );
    let batch_context = context.clone();
    let worker = tokio::spawn(async move {
        batch_context
            .handle_index_full_text_batch(vec![
                IndexChunkRequest::new(edited_artifact, edited_chunk),
                IndexChunkRequest::new(added_artifact, added_chunk),
            ])
            .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(5), entered_rx).await??;
    // The runtime has buffered all three mutations and reached the one publish
    // boundary. Search remains on the prior reader snapshot while publication
    // is pending; it must neither commit nor expose only part of the batch.
    let old_edit_during_ingestion = search_hits(&index, "previouseditterm");
    let old_delete_during_ingestion = search_hits(&index, "removedfromsearchterm");
    let edited_text_during_ingestion = search_hits(&index, "currenteditterm");
    let added_text_during_ingestion = search_hits(&index, "newartifactterm");
    assert!(
        matches!(input_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "completion feedback must wait for publication"
    );
    gate.release();

    let batch_result = worker.await?;
    assert!(
        batch_result.is_ok(),
        "full-text batch failed: {batch_result:?}"
    );
    assert_eq!(gate.commits.load(Ordering::SeqCst), 1);
    assert_eq!(old_edit_during_ingestion?.len(), 1);
    assert_eq!(old_delete_during_ingestion?.len(), 1);
    assert!(edited_text_during_ingestion?.is_empty());
    assert!(added_text_during_ingestion?.is_empty());

    for _ in 0..2 {
        let input = input_rx
            .recv()
            .await
            .ok_or("full-text completion channel closed before both artifacts completed")?;
        let DomainInput::FullTextIndexCompleted(completion) = &input else {
            return Err(format!("unexpected indexing input: {input:?}").into());
        };
        let completed_artifact = completion.artifact_id;

        // Receiving a completion means the batch is durable and searchable,
        // even though the domain has not yet consumed the feedback input.
        assert_eq!(search_hits(&index, "previouseditterm")?.len(), 0);
        assert_eq!(search_hits(&index, "removedfromsearchterm")?.len(), 0);
        assert_eq!(search_hits(&index, "currenteditterm")?.len(), 1);
        assert_eq!(search_hits(&index, "newartifactterm")?.len(), 1);

        state.write().await.apply_input(input)?;
        assert_eq!(
            state.read().await.artifacts[&completed_artifact].index_status,
            IndexStatus::Indexed
        );
    }

    drop(context);
    drop(adapters);
    drop(index);
    let reopened = TantivyFullTextIndex::open(directory.path())?;
    assert_eq!(search_hits(&reopened, "previouseditterm")?.len(), 0);
    assert_eq!(search_hits(&reopened, "removedfromsearchterm")?.len(), 0);
    assert_eq!(search_hits(&reopened, "currenteditterm")?.len(), 1);
    assert_eq!(search_hits(&reopened, "newartifactterm")?.len(), 1);
    Ok(())
}
