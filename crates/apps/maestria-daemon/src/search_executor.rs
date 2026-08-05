#[path = "search_runtime_construction.rs"]
mod construction;
#[path = "search_runtime_engine.rs"]
mod engine;
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
    prepare_search_runtime_read_only_for_federation,
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
use maestria_retrieval::adapters::VisualGenerationCapability;
use maestria_retrieval::{
    CandidateReranker, RepositoryExecutionPolicy, SearchPlannerContext, VisualExecutionPolicy,
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

    /// Produces a request-bound runtime that cannot materialize graph
    /// relations before authorization. Federation deliberately degrades this
    /// lane until the graph port supports pre-materialization filtering.
    pub fn without_graph_expansion(&self) -> Self {
        let mut runtime = self.clone();
        runtime.graph_index = None;
        runtime.persist_learned_sparse_observations = false;
        runtime
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

    fn execute_pre_authorized_blocking(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let engine = self.retrieval_engine()?;
        let plan = engine
            .plan(query, limit, &self.planner_context())
            .map_err(anyhow::Error::new)?
            .confine_to_scope(self.scope_id)
            .map_err(anyhow::Error::new)?;
        let outcome = tokio::runtime::Handle::current()
            .block_on(engine.search_pre_authorized(&plan, authorization))
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

    /// Executes a provider-composed authorization context without rebuilding
    /// policy in any retrieval lane.
    ///
    /// # Cancellation
    /// Cancelling the returned future does not abort the blocking search worker; the spawned
    /// blocking task continues until completion.
    pub async fn execute_pre_authorized(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || {
            runtime.execute_pre_authorized_blocking(query, limit, authorization)
        })
        .await
        .map_err(|error| anyhow!("search worker failed: {error}"))?
    }
}
