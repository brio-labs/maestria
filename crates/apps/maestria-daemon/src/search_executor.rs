use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    CorpusSnapshotId, DomainEventEnvelope, IndexGenerationId, IndexStatus, KernelState,
    RepresentationName, RetrievalModelFingerprint, SearchOutcome, SearchPlan,
};
use maestria_governance::scan_secrets;
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EmbeddingProvider, EventLog,
    EvidenceRepository, FullTextIndex, GraphIndex, IndexedCard, SearchKnowledgeExecutor,
    VectorIndex,
};
use maestria_retrieval::adapters::{
    CardRetriever, CardRetrieverParts, DenseChunkRetriever, DenseChunkRetrieverParts,
    EvidenceOutcomeEvaluator, HierarchyGraphExpander, HierarchyGraphExpanderParts,
    LexicalChunkRetriever, LexicalChunkRetrieverParts,
};
use maestria_retrieval::{
    CandidateRetriever, FixedKRrf, HybridExecutionPolicy, RetrievalEngine, SearchPlannerContext,
};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;
pub(crate) struct SearchRuntimeParts {
    pub(crate) artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub(crate) cards: Arc<dyn CardRepository + Send + Sync>,
    pub(crate) chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub(crate) evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub(crate) search_index: Arc<dyn FullTextIndex + Send + Sync>,
    pub(crate) blobs: Arc<dyn BlobStore + Send + Sync>,
    pub(crate) vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    pub(crate) graph_index: Option<Arc<dyn GraphIndex + Send + Sync>>,
    pub(crate) event_log: Arc<SqliteStore>,
    pub(crate) primary_generation: IndexGenerationId,
    pub(crate) dense_generation: Option<IndexGenerationId>,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
}

/// One immutable set of repositories, generations, and indexes used for a search request.
///
/// The daemon owns construction so direct CLI search, explain, and background
/// search effects cannot drift into separate retrieval implementations.
#[derive(Clone)]
pub struct SearchRuntime {
    pub(crate) artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub(crate) cards: Arc<dyn CardRepository + Send + Sync>,
    pub(crate) chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub(crate) evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub(crate) search_index: Arc<dyn FullTextIndex + Send + Sync>,
    pub(crate) blobs: Arc<dyn BlobStore + Send + Sync>,
    pub(crate) vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    pub(crate) graph_index: Option<Arc<dyn GraphIndex + Send + Sync>>,
    pub(crate) event_log: Arc<SqliteStore>,
    pub(crate) embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
    pub(crate) retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    pub(crate) primary_generation: IndexGenerationId,
    pub(crate) dense_generation: Option<IndexGenerationId>,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) fingerprint: RetrievalModelFingerprint,
}

impl SearchRuntime {
    pub(crate) fn from_parts(
        parts: SearchRuntimeParts,
        embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Result<Self> {
        let fingerprint =
            RetrievalModelFingerprint::new("maestria-core:deterministic-v1".to_string())
                .map_err(|error| anyhow!("create retrieval fingerprint: {error}"))?;
        Ok(Self {
            artifacts: parts.artifacts,
            cards: parts.cards,
            chunks: parts.chunks,
            evidence: parts.evidence,
            search_index: parts.search_index,
            blobs: parts.blobs,
            vector_index: parts.vector_index,
            graph_index: parts.graph_index,
            event_log: parts.event_log,
            embedding_provider,
            retrieval_policy,
            primary_generation: parts.primary_generation,
            dense_generation: parts.dense_generation,
            corpus_snapshot: parts.corpus_snapshot,
            fingerprint,
        })
    }

    pub fn append_events(
        &self,
        events: impl IntoIterator<Item = DomainEventEnvelope>,
    ) -> Result<()> {
        for event in events {
            EventLog::append(self.event_log.as_ref(), event)
                .map_err(|error| anyhow!("append search event: {error}"))?;
        }
        Ok(())
    }

    fn retrieval_engine(&self) -> RetrievalEngine {
        let card = Arc::new(CardRetriever::new(
            CardRetrieverParts {
                index: self.search_index.clone(),
                artifacts: self.artifacts.clone(),
                cards: self.cards.clone(),
                chunks: self.chunks.clone(),
                evidence: self.evidence.clone(),
                blobs: self.blobs.clone(),
            },
            self.retrieval_policy.clone(),
            self.primary_generation,
        ));
        let lexical = Arc::new(LexicalChunkRetriever::new(
            LexicalChunkRetrieverParts {
                index: self.search_index.clone(),
                artifacts: self.artifacts.clone(),
                chunks: self.chunks.clone(),
                evidence: self.evidence.clone(),
                blobs: self.blobs.clone(),
            },
            self.retrieval_policy.clone(),
            self.primary_generation,
        ));
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = vec![card, lexical];
        if let (Some(vector_index), Some(provider), Some(generation)) = (
            self.vector_index.clone(),
            self.embedding_provider.clone(),
            self.dense_generation,
        ) {
            retrievers.push(Arc::new(DenseChunkRetriever::new(
                DenseChunkRetrieverParts {
                    index: vector_index,
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                    embedding_provider: provider,
                },
                self.retrieval_policy.clone(),
                generation,
            )));
        }
        let mut engine = RetrievalEngine::new(
            retrievers,
            Arc::new(EvidenceOutcomeEvaluator::new(self.evidence.clone())),
        )
        .with_fusion(Arc::new(FixedKRrf::new(60)));
        if let Some(graph) = self.graph_index.clone() {
            engine = engine.with_expander(Arc::new(HierarchyGraphExpander::new(
                HierarchyGraphExpanderParts {
                    graph,
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.retrieval_policy.clone(),
            )));
        }
        engine.with_hybrid_policy(HybridExecutionPolicy::Shadow)
    }

    fn planner_context(&self) -> SearchPlannerContext {
        SearchPlannerContext {
            corpus_snapshot: self.corpus_snapshot,
            primary_generation: self.primary_generation,
            fingerprint: self.fingerprint.clone(),
        }
    }

    fn execute_plan_blocking(&self, plan: SearchPlan) -> Result<SearchOutcome> {
        let engine = self.retrieval_engine();
        tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(|error| anyhow!(error.to_string()))
    }

    fn execute_search_blocking(
        &self,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let engine = self.retrieval_engine();
        let plan = engine
            .plan(query, limit, &self.planner_context())
            .map_err(|error| anyhow!(error.to_string()))?;
        let outcome = tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok((plan, outcome))
    }

    /// Build and execute the same plan used by daemon search effects.
    pub async fn execute(
        &self,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || runtime.execute_search_blocking(query, limit))
            .await
            .map_err(|error| anyhow!("search worker failed: {error}"))?
    }
}

impl SearchKnowledgeExecutor for SearchRuntime {
    fn search(
        &self,
        plan: SearchPlan,
    ) -> Pin<Box<dyn Future<Output = Result<SearchOutcome, maestria_ports::PortError>> + Send + '_>>
    {
        let runtime = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || runtime.execute_plan_blocking(plan))
                .await
                .map_err(|error| maestria_ports::PortError::Internal {
                    message: format!("search worker failed: {error}"),
                })?
                .map_err(|error| maestria_ports::PortError::Internal {
                    message: error.to_string(),
                })
        })
    }
}

/// Construct the one search runtime used by CLI search and explain.
pub fn prepare_search_runtime(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
) -> Result<Arc<SearchRuntime>> {
    let sqlite_store = Arc::new(
        SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?,
    );
    let blob_store = Arc::new(
        FsBlobStore::open(&layout.blobs_dir)
            .with_context(|| format!("open blob store {}", layout.blobs_dir.display()))?,
    );
    let search_index = Arc::new(
        TantivyFullTextIndex::open(&layout.full_text_index_dir).with_context(|| {
            format!(
                "open full-text index {}",
                layout.full_text_index_dir.display()
            )
        })?,
    );
    ensure_search_index(&search_index, state)?;
    let embedding_provider = crate::build_embedding_provider(manifest, state)?;
    let vector_index: Option<Arc<dyn VectorIndex + Send + Sync>> = if embedding_provider.is_some() {
        Some(Arc::new(
            SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db")).with_context(
                || format!("open vector index {}", layout.vector_index_dir.display()),
            )?,
        ))
    } else {
        None
    };
    if let (Some(provider), Some(vector_index)) =
        (embedding_provider.as_deref(), vector_index.as_deref())
    {
        let model = manifest
            .embeddings
            .as_ref()
            .filter(|config| config.enabled)
            .map(|config| config.model.as_str());
        if let Err(error) =
            crate::reconcile_vector_projection(state, vector_index, Some(provider), model)
        {
            eprintln!("dense retrieval unavailable; using lexical fallback: {error}");
        }
    }
    let graph_index = Arc::new(
        SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?,
    );
    crate::reconcile_graph_projection(state, &*graph_index)
        .with_context(|| "reconcile graph projection for search")?;
    let lexical = state
        .index_generations
        .get_active(&RepresentationName::new("lexical_text_v1"))
        .ok_or_else(|| anyhow!("active lexical retrieval generation is missing"))?;
    let dense_generation = state
        .index_generations
        .get_active(&RepresentationName::new("dense_text_v1"))
        .map(|generation| generation.id);
    let parts = SearchRuntimeParts {
        artifacts: sqlite_store.clone(),
        cards: sqlite_store.clone(),
        chunks: sqlite_store.clone(),
        evidence: sqlite_store.clone(),
        search_index,
        blobs: blob_store,
        vector_index,
        event_log: sqlite_store.clone(),
        graph_index: Some(graph_index),
        primary_generation: lexical.id,
        dense_generation,
        corpus_snapshot: lexical.corpus_snapshot,
    };
    Ok(Arc::new(SearchRuntime::from_parts(
        parts,
        embedding_provider,
        retrieval_policy,
    )?))
}

fn ensure_search_index(search_index: &TantivyFullTextIndex, state: &KernelState) -> Result<()> {
    if !search_index.needs_card_rebuild()? {
        return Ok(());
    }
    let cards: Vec<IndexedCard> = state
        .cards
        .values()
        .filter(|card| {
            state
                .artifacts
                .get(&card.artifact_id)
                .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
                && scan_secrets(&card.title).is_clean()
                && scan_secrets(&card.body).is_clean()
        })
        .map(|card| IndexedCard {
            artifact_id: card.artifact_id,
            card_id: card.id,
            title: card.title.clone(),
            body: card.body.clone(),
        })
        .collect();
    search_index.index_cards(cards)?;
    search_index.complete_card_rebuild()?;
    Ok(())
}
