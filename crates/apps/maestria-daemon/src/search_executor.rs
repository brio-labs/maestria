#[path = "search_runtime_construction.rs"]
mod construction;
#[path = "search_executor_projection.rs"]
mod projection;
#[path = "repository_code_loader.rs"]
mod repository_code_loader;
pub use construction::{
    prepare_search_runtime, prepare_search_runtime_read_only,
    prepare_search_runtime_read_only_with_repository_policy,
    prepare_search_runtime_with_repository_policy,
};
pub(crate) use repository_code_loader::load_repository_code_index_with_exclusions;
#[cfg(test)]
#[path = "search_executor_tests.rs"]
mod tests;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_code_intel::RepositoryCodeIndex;
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    ArtifactVersionId, CorpusSnapshotId, DomainEvent, DomainEventEnvelope, IndexGenerationId,
    KernelState, RepresentationName, RetrievalModelFingerprint, SearchOutcome, SearchPlan,
};
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EmbeddingProvider, EventFilter,
    EventLog, EvidenceRepository, FullTextIndex, GraphIndex, SearchKnowledgeExecutor, VectorIndex,
};
use maestria_retrieval::adapters::{
    CardRetriever, CardRetrieverParts, CodeIntelRetriever, CodeIntelRetrieverParts,
    CurrentVersionFilter, DenseChunkRetriever, DenseChunkRetrieverParts, EvidenceOutcomeEvaluator,
    HierarchyGraphExpander, HierarchyGraphExpanderParts, LexicalChunkRetriever,
    LexicalChunkRetrieverParts,
};
use maestria_retrieval::{
    CandidateRetriever, FixedKRrf, HybridExecutionPolicy, RepositoryExecutionPolicy,
    RetrievalEngine, SearchPlannerContext,
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
    pub(crate) repository_code_index: Option<Arc<RepositoryCodeIndex>>,
    pub(crate) repository_execution_policy: RepositoryExecutionPolicy,
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
    pub(crate) repository_code_index: Option<Arc<RepositoryCodeIndex>>,
    pub(crate) repository_execution_policy: RepositoryExecutionPolicy,
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
                .map_err(|error| anyhow!(error.to_string()))?;
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
            repository_code_index: parts.repository_code_index,
            repository_execution_policy: parts.repository_execution_policy,
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

    fn current_artifact_versions(&self) -> Result<BTreeSet<ArtifactVersionId>> {
        let events = EventLog::scan(self.event_log.as_ref(), EventFilter { artifact_id: None })
            .map_err(|error| {
                anyhow!("scan parser history for current artifact versions: {error}")
            })?;
        let mut latest_by_path = BTreeMap::new();
        for envelope in events {
            if let DomainEvent::ParserStarted {
                artifact_id,
                source_path,
                ..
            } = envelope.event
            {
                latest_by_path.insert(source_path, ArtifactVersionId::new(artifact_id.value()));
            }
        }
        Ok(latest_by_path.into_values().collect())
    }

    fn retrieval_engine(&self) -> Result<RetrievalEngine> {
        let active_versions = self.current_artifact_versions()?;
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
                self.retrieval_policy.clone(),
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
                self.retrieval_policy.clone(),
                self.primary_generation,
            )),
            active_versions.clone(),
        ));
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = vec![card, lexical];
        if let Some(index) = self.repository_code_index.clone() {
            retrievers.push(Arc::new(CodeIntelRetriever::new(
                CodeIntelRetrieverParts { index },
                self.retrieval_policy.clone(),
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
                    self.retrieval_policy.clone(),
                    generation,
                )),
                active_versions,
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
        Ok(engine
            .with_hybrid_policy(HybridExecutionPolicy::Shadow)
            .with_repository_execution_policy(self.repository_execution_policy.clone()))
    }

    fn planner_context(&self) -> SearchPlannerContext {
        SearchPlannerContext {
            corpus_snapshot: self.corpus_snapshot,
            primary_generation: self.primary_generation,
            fingerprint: self.fingerprint.clone(),
        }
    }

    fn execute_plan_blocking(&self, plan: SearchPlan) -> Result<SearchOutcome> {
        let engine = self.retrieval_engine()?;
        tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(|error| anyhow!(error.to_string()))
    }

    fn execute_search_blocking(
        &self,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let engine = self.retrieval_engine()?;
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
