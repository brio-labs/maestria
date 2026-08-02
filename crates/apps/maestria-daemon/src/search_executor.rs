#[path = "search_runtime_construction.rs"]
mod construction;
#[path = "search_executor_port.rs"]
mod port;
#[path = "search_executor_projection.rs"]
pub(crate) mod projection;
#[path = "repository_code_loader.rs"]
mod repository_code_loader;
#[path = "search_visual_runtime.rs"]
mod visual_runtime;
pub use construction::{
    prepare_search_runtime, prepare_search_runtime_read_only,
    prepare_search_runtime_read_only_with_repository_policy,
    prepare_search_runtime_with_repository_policy,
};
pub(crate) use repository_code_loader::load_repository_code_index_with_exclusions;
#[cfg(test)]
#[path = "search_executor_tests.rs"]
mod tests;
use std::{collections::BTreeSet, sync::Arc};

use anyhow::{Result, anyhow};
use maestria_code_intel::RepositoryCodeIndex;
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    ArtifactVersionId, CorpusSnapshotId, DomainEventEnvelope, IndexGenerationId, KernelState,
    RetrievalModelFingerprint, SearchOutcome, SearchPlan,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EmbeddingProvider, EventFilter,
    EventLog, EvidenceRepository, FullTextIndex, GraphIndex, VectorIndex,
};
use maestria_retrieval::adapters::{
    CardRetriever, CardRetrieverParts, CodeIntelRetriever, CodeIntelRetrieverParts,
    CodeIntelSecurityResolver, CodeIntelSecurityResolverParts, CurrentVersionFilter,
    DenseChunkRetriever, DenseChunkRetrieverParts, EvidenceOutcomeEvaluator,
    HierarchyGraphExpander, HierarchyGraphExpanderParts, LexicalChunkRetriever,
    LexicalChunkRetrieverParts, VisualGenerationCapability, VisualPageRegionRetriever,
    VisualPageRegionRetrieverParts,
};
use maestria_retrieval::{
    CandidateReranker, CandidateRetriever, FixedKRrf, HybridExecutionPolicy,
    RepositoryExecutionPolicy, RetrievalEngine, SearchPlannerContext, VisualExecutionPolicy,
};
use maestria_storage_sqlite::SqliteStore;

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
    pub(crate) repository_code_index: Option<Arc<RepositoryCodeIndex>>,
    pub(crate) repository_execution_policy: RepositoryExecutionPolicy,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) scope_id: maestria_domain::ScopeId,
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
    pub(crate) visual_vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    pub(crate) graph_index: Option<Arc<dyn GraphIndex + Send + Sync>>,
    pub(crate) event_log: Arc<SqliteStore>,
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
    pub(crate) visual_execution_policy: VisualExecutionPolicy,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) scope_id: maestria_domain::ScopeId,
    pub(crate) fingerprint: RetrievalModelFingerprint,
}

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
            embedding_provider,
            reranker: None,
            visual_embedding_provider: None,
            visual_generation: None,
            retrieval_policy,
            primary_generation: parts.primary_generation,
            dense_generation: parts.dense_generation,
            repository_code_index: parts.repository_code_index,
            repository_execution_policy: parts.repository_execution_policy,
            visual_execution_policy: VisualExecutionPolicy::Shadow,
            corpus_snapshot: parts.corpus_snapshot,
            scope_id: parts.scope_id,
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

    fn domain_events(&self) -> Result<Vec<DomainEventEnvelope>> {
        EventLog::scan(self.event_log.as_ref(), EventFilter { artifact_id: None })
            .map_err(|error| anyhow!("scan domain history for retrieval: {error}"))
    }

    pub(crate) fn current_artifact_versions(&self) -> Result<BTreeSet<ArtifactVersionId>> {
        let events = self.domain_events()?;
        Ok(reconcile_active_versions(&events))
    }

    fn retrieval_engine(&self) -> Result<RetrievalEngine> {
        let events = self.domain_events()?;
        let active_versions = reconcile_active_versions(&events);
        let card: Arc<dyn CandidateRetriever> = Arc::new(CurrentVersionFilter::new(
            Arc::new(CardRetriever::new(
                CardRetrieverParts {
                    index: self.search_index.clone(),
                    artifacts: self.artifacts.clone(),
                    cards: self.cards.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.primary_generation,
            )),
            active_versions.clone(),
        ));
        let lexical: Arc<dyn CandidateRetriever> = Arc::new(CurrentVersionFilter::new(
            Arc::new(LexicalChunkRetriever::new(
                LexicalChunkRetrieverParts {
                    index: self.search_index.clone(),
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.primary_generation,
            )),
            active_versions.clone(),
        ));
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = vec![card, lexical];
        if let Some(index) = self.repository_code_index.clone() {
            let security = CodeIntelSecurityResolver::from_events(
                CodeIntelSecurityResolverParts {
                    artifacts: self.artifacts.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                &events,
            )
            .map_err(|error| anyhow!("prepare repository code security resolver: {error}"))?;
            retrievers.push(Arc::new(CodeIntelRetriever::new(
                CodeIntelRetrieverParts { index, security },
                self.primary_generation,
            )));
        }
        if let (Some(vector_index), Some(provider), Some(generation)) = (
            self.vector_index.clone(),
            self.embedding_provider.clone(),
            self.dense_generation,
        ) {
            retrievers.push(Arc::new(CurrentVersionFilter::new(
                Arc::new(DenseChunkRetriever::new(
                    DenseChunkRetrieverParts {
                        index: vector_index,
                        artifacts: self.artifacts.clone(),
                        chunks: self.chunks.clone(),
                        evidence: self.evidence.clone(),
                        blobs: self.blobs.clone(),
                        embedding_provider: provider,
                    },
                    generation,
                )),
                active_versions.clone(),
            )));
        }
        if let Some(retriever) = self.visual_retriever(active_versions) {
            retrievers.push(retriever);
        }
        let mut engine = RetrievalEngine::new(
            retrievers,
            Arc::new(EvidenceOutcomeEvaluator::new(self.evidence.clone())),
            self.retrieval_policy.clone(),
        )
        .with_fusion(Arc::new(FixedKRrf::new(60)))
        .with_learned_sparse_observation_repository(self.event_log.clone());
        if let Some(reranker) = self.reranker.clone() {
            engine = engine.with_visual_reranker(reranker);
        }
        if let Some(graph) = self.graph_index.clone() {
            engine = engine.with_expander(Arc::new(HierarchyGraphExpander::new(
                HierarchyGraphExpanderParts {
                    graph,
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
            )));
        }
        Ok(engine
            .with_hybrid_policy(HybridExecutionPolicy::Shadow)
            .with_repository_execution_policy(self.repository_execution_policy.clone())
            .with_visual_execution_policy(self.visual_execution_policy.clone()))
    }

    fn planner_context(&self) -> SearchPlannerContext {
        SearchPlannerContext {
            corpus_snapshot: self.corpus_snapshot,
            primary_generation: self.primary_generation,
            fingerprint: self.fingerprint.clone(),
            scope: Some(self.scope_id),
        }
    }

    fn execute_plan_blocking(&self, plan: SearchPlan) -> Result<SearchOutcome> {
        // R43: direct CLI/API searches enforce the same scope dimension as the
        // runtime effect path; the shared transition rejects out-of-scope plans.
        let plan = plan
            .confine_to_scope(self.scope_id)
            .map_err(anyhow::Error::new)?;
        let engine = self.retrieval_engine()?;
        tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(anyhow::Error::new)
    }

    fn execute_search_blocking(
        &self,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let engine = self.retrieval_engine()?;
        // The plan is built already confined to the instance scope; the typed
        // transition is kept as a guard so scope enforcement cannot drift
        // (R28/R43).
        let plan = engine
            .plan(query, limit, &self.planner_context())
            .map_err(anyhow::Error::new)?
            .confine_to_scope(self.scope_id)
            .map_err(anyhow::Error::new)?;
        let outcome = tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(anyhow::Error::new)?;
        Ok((plan, outcome))
    }

    /// Build and execute the same plan used by daemon search effects.
    ///
    /// # Cancellation
    /// Cancelling the returned future does not abort the blocking search worker; the spawned
    /// blocking task continues until completion.
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
