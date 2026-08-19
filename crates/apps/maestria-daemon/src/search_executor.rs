#[path = "search_runtime_construction.rs"]
mod construction;
#[path = "search_executor_dispatch.rs"]
mod dispatch;
#[path = "search_runtime_engine.rs"]
mod engine;
#[path = "search_runtime_parts.rs"]
pub(crate) mod parts;
#[path = "search_executor_port.rs"]
mod port;
#[path = "search_executor_projection.rs"]
pub(crate) mod projection;
pub(crate) use construction::load_repository_code_index_with_exclusions;
pub use construction::{
    prepare_search_runtime, prepare_search_runtime_read_only,
    prepare_search_runtime_read_only_for_federation,
    prepare_search_runtime_read_only_with_repository_policy,
    prepare_search_runtime_with_repository_policy,
};
#[cfg(test)]
#[path = "search_executor_tests.rs"]
mod tests;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_code_intel::RepositoryCodeIndex;
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    CorpusSnapshotId, DomainEventEnvelope, IndexGenerationId, KernelState,
    RetrievalModelFingerprint,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EmbeddingProvider, EventFilter,
    EventLog, EvidenceRepository, FullTextIndex, GraphIndex, VectorIndex,
};
use maestria_retrieval::adapters::VisualGenerationCapability;
use maestria_retrieval::{
    CandidateReranker, CandidateRetriever, RepositoryExecutionPolicy, SearchPlannerContext,
    VisualExecutionPolicy,
};
use maestria_storage_sqlite::SqliteStore;
use parking_lot::RwLock;

pub(crate) type EngineSignature = (
    usize,
    Option<u64>,
    IndexGenerationId,
    Option<IndexGenerationId>,
    CorpusSnapshotId,
);
pub(crate) type CachedEngine = (EngineSignature, Arc<maestria_retrieval::RetrievalEngine>);
pub(crate) type EngineCache = Arc<RwLock<Option<CachedEngine>>>;

/// One immutable set of repositories, generations, and indexes used for a search request.
///
/// The daemon owns construction so direct CLI search, explain, and background
/// search effects cannot drift into separate retrieval implementations.
pub struct SearchRuntime {
    pub(crate) artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub(crate) cards: Arc<dyn CardRepository + Send + Sync>,
    pub(crate) chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub(crate) evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub(crate) search_index: Arc<dyn FullTextIndex + Send + Sync>,
    pub(crate) blobs: Arc<dyn BlobStore + Send + Sync>,
    pub(crate) vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    pub(crate) visual_vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    pub(crate) graph_index: Option<Arc<dyn GraphIndex + Send + Sync>>,
    pub(crate) event_log: Arc<SqliteStore>,
    pub(crate) persist_learned_sparse_observations: bool,
    pub(crate) embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
    pub(crate) reranker: Option<Arc<dyn CandidateReranker>>,
    pub(crate) retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    pub(crate) primary_generation: IndexGenerationId,
    pub(crate) dense_generation: Option<IndexGenerationId>,
    pub(crate) visual_embedding_provider:
        Option<Arc<dyn maestria_ports::VisualEmbeddingProvider + Send + Sync>>,
    pub(crate) visual_generation: Option<VisualGenerationCapability>,
    pub(crate) repository_code_index: Option<Arc<RepositoryCodeIndex>>,
    pub(crate) repository_execution_policy: RepositoryExecutionPolicy,
    pub(crate) hybrid_execution_policy: maestria_retrieval::HybridExecutionPolicy,
    pub(crate) visual_execution_policy: VisualExecutionPolicy,
    pub(crate) learned_sparse_execution_policy: maestria_retrieval::LearnedSparseExecutionPolicy,
    pub(crate) sparse_retriever: Option<Arc<dyn CandidateRetriever>>,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) scope_id: maestria_domain::ScopeId,
    pub(crate) fingerprint: RetrievalModelFingerprint,
    pub(crate) engine_cache: EngineCache,
}

pub(crate) use parts::SearchRuntimeParts;
pub(crate) use projection::reconcile_active_versions;

impl SearchRuntime {
    pub(crate) fn from_parts(
        parts: SearchRuntimeParts,
        embedding_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Result<Self> {
        let fingerprint =
            RetrievalModelFingerprint::new("maestria-core:deterministic-v1".to_string())
                .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self {
            artifacts: parts.artifacts,
            cards: parts.cards,
            chunks: parts.chunks,
            evidence: parts.evidence,
            search_index: parts.search_index,
            blobs: parts.blobs,
            vector_index: parts.vector_index,
            visual_vector_index: None,
            graph_index: parts.graph_index,
            event_log: parts.event_log,
            persist_learned_sparse_observations: true,
            embedding_provider,
            reranker: None,
            visual_embedding_provider: None,
            visual_generation: None,
            retrieval_policy,
            primary_generation: parts.primary_generation,
            dense_generation: parts.dense_generation,
            repository_code_index: parts.repository_code_index,
            repository_execution_policy: parts.repository_execution_policy,
            hybrid_execution_policy: parts.hybrid_execution_policy,
            visual_execution_policy: VisualExecutionPolicy::Shadow,
            learned_sparse_execution_policy: parts.learned_sparse_execution_policy,
            sparse_retriever: parts.sparse_retriever,
            corpus_snapshot: parts.corpus_snapshot,
            scope_id: parts.scope_id,
            fingerprint,
            engine_cache: Arc::new(RwLock::new(None)),
        })
    }

    pub(crate) fn assemble(
        layout: &InstanceLayout,
        state: &KernelState,
        manifest: &InstanceManifest,
        retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
        repository_execution_policy: RepositoryExecutionPolicy,
        allow_projection_writes: bool,
        federation_read_only: bool,
    ) -> Result<Arc<Self>> {
        use crate::projection_open::{
            open_base_stores, open_base_stores_read_only, open_full_text_index, open_graph_index,
            open_vector_index, reconcile_vector_projection, resolve_index_generations,
        };
        let (sqlite_store, blob_store) = if federation_read_only {
            open_base_stores_read_only(layout)?
        } else {
            open_base_stores(layout)?
        };
        let search_index = open_full_text_index(
            layout,
            state,
            allow_projection_writes,
            allow_projection_writes,
        )?;
        let repository_code_index =
            crate::search_executor::load_repository_code_index_with_exclusions(
                layout,
                Some(manifest),
            )
            .context("load repository code index")?;
        let embedding_provider = if federation_read_only {
            None
        } else {
            crate::vector_startup::build_embedding_provider(manifest, state)?
        };
        let vector_index = if federation_read_only {
            None
        } else {
            open_vector_index(layout, embedding_provider.is_some())?
        };
        reconcile_vector_projection(
            state,
            manifest,
            &embedding_provider,
            &vector_index,
            allow_projection_writes,
        );
        let graph_index: Option<Arc<dyn GraphIndex + Send + Sync>> = if federation_read_only {
            None
        } else {
            Some(open_graph_index(layout, state, allow_projection_writes)?)
        };
        let (primary_generation, corpus_snapshot, dense_generation) =
            resolve_index_generations(state)?;
        let (hybrid_execution_policy, learned_sparse_execution_policy, sparse_retriever) =
            crate::runtime_construction::search_lane_bundle(
                state,
                manifest,
                sqlite_store.clone(),
                blob_store.clone(),
            );
        let parts = SearchRuntimeParts {
            artifacts: sqlite_store.clone(),
            cards: sqlite_store.clone(),
            chunks: sqlite_store.clone(),
            evidence: sqlite_store.clone(),
            search_index,
            blobs: blob_store,
            vector_index,
            graph_index,
            event_log: sqlite_store,
            primary_generation,
            dense_generation,
            repository_code_index,
            repository_execution_policy,
            hybrid_execution_policy,
            learned_sparse_execution_policy,
            sparse_retriever,
            corpus_snapshot,
            scope_id: maestria_domain::DEFAULT_INSTANCE_SCOPE_ID,
        };
        Ok(Arc::new(Self::from_parts(
            parts,
            embedding_provider,
            retrieval_policy,
        )?))
    }

    pub fn append_events(
        &self,
        events: impl IntoIterator<Item = DomainEventEnvelope>,
    ) -> Result<()> {
        for event in events {
            EventLog::append(self.event_log.as_ref(), event)
                .map_err(|error| anyhow!("append search event: {error}"))?;
        }
        // Invalidate the cached engine: the event count changed.
        *self.engine_cache.write() = None;
        Ok(())
    }

    fn domain_events(&self) -> Result<Vec<DomainEventEnvelope>> {
        EventLog::scan(self.event_log.as_ref(), EventFilter { artifact_id: None })
            .map_err(|error| anyhow!("scan domain history for retrieval: {error}"))
    }

    /// Produces a request-bound runtime that cannot materialize graph
    /// relations before authorization. Federation deliberately degrades this
    /// lane until the graph port supports pre-materialization filtering.
    pub fn without_graph_expansion(&self) -> Self {
        let mut runtime = self.clone();
        runtime.graph_index = None;
        runtime.persist_learned_sparse_observations = false;
        // The cloned runtime serves a different lane set; its cache must not be shared.
        runtime.engine_cache = Arc::new(RwLock::new(None));
        runtime
    }

    pub(crate) fn planner_context(&self) -> SearchPlannerContext {
        SearchPlannerContext {
            corpus_snapshot: self.corpus_snapshot,
            primary_generation: self.primary_generation,
            fingerprint: self.fingerprint.clone(),
            scope: Some(self.scope_id),
        }
    }

    pub(crate) fn engine_signature(&self, events: &[DomainEventEnvelope]) -> EngineSignature {
        let last = events.last().map(|e| e.id.value());
        (
            events.len(),
            last,
            self.primary_generation,
            self.dense_generation,
            self.corpus_snapshot,
        )
    }

    pub(crate) fn cached_retrieval_engine(
        &self,
    ) -> Result<Arc<maestria_retrieval::RetrievalEngine>> {
        let events = self.domain_events()?;
        let sig = self.engine_signature(&events);
        {
            let cache = self.engine_cache.read();
            if let Some((cached_sig, engine)) = cache.as_ref()
                && *cached_sig == sig
            {
                return Ok(engine.clone());
            }
        }
        // Build fresh engine (single scan shared by base retrievers).
        let engine = self.retrieval_engine()?;
        let engine = Arc::new(engine);
        *self.engine_cache.write() = Some((sig, engine.clone()));
        Ok(engine)
    }
}
