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
#[path = "search_executor/runtime_setup.rs"]
mod runtime_setup;
#[path = "search_executor/snapshot_refresh.rs"]
mod snapshot_refresh;
pub(crate) use construction::load_repository_code_index_with_exclusions;
pub use construction::{
    prepare_search_runtime, prepare_search_runtime_read_only,
    prepare_search_runtime_read_only_for_federation,
    prepare_search_runtime_read_only_with_repository_policy,
    prepare_search_runtime_with_repository_policy,
};
pub(crate) use dispatch::path_is_within_allowed_roots;
#[cfg(test)]
#[path = "search_executor_tests.rs"]
mod tests;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use parking_lot::RwLock;
use sillage_code_intel::RepositoryCodeIndex;
use sillage_core::{InstanceLayout, InstanceManifest};
use sillage_domain::{
    ActiveSourceVersions, CorpusSnapshotId, DomainEventEnvelope, IndexGenerationId,
    RetrievalModelFingerprint,
};
use sillage_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EmbeddingProvider, EventFilter,
    EventLog, EvidenceRepository, FullTextIndex, GraphIndex, VectorIndex,
};
use sillage_retrieval::adapters::VisualGenerationCapability;
use sillage_retrieval::{
    CandidateReranker, CandidateRetriever, RepositoryExecutionPolicy, SearchPlannerContext,
    VisualExecutionPolicy,
};
use sillage_storage_sqlite::SqliteStore;

pub(crate) type EngineSignature = (
    usize,
    Option<u64>,
    IndexGenerationId,
    Option<IndexGenerationId>,
    CorpusSnapshotId,
);

pub(crate) type CachedEngine = (EngineSignature, Arc<sillage_retrieval::RetrievalEngine>);
pub(crate) type EngineCache = Arc<RwLock<Option<CachedEngine>>>;
// Final source filter keyed by the current manifest and exact consumer root grant.
type InteractiveApprovedCache = Arc<
    RwLock<
        Option<(
            InstanceManifest,
            Option<Arc<[PathBuf]>>,
            sillage_retrieval::CandidateSourceFilter,
        )>,
    >,
>;
#[derive(Clone)]
pub(crate) struct InteractiveSnapshot {
    revision: i64,
    engine: Arc<sillage_retrieval::RetrievalEngine>,
    sources: Arc<ActiveSourceVersions>,
    projection: Option<Arc<sillage_domain::SourceProjection>>,
    approved: InteractiveApprovedCache,
}
pub(crate) type InteractiveCache = Arc<RwLock<Option<InteractiveSnapshot>>>;
/// A candidate filename match retained only inside the provider until its
/// active version, root approval, and on-disk freshness are revalidated.
pub(crate) struct InteractivePathCandidate {
    pub(crate) path: std::path::PathBuf,
    artifact_id: sillage_domain::ArtifactId,
    artifact_version: sillage_domain::ArtifactVersionId,
    content_hash: sillage_domain::ContentHash,
}

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
    pub(crate) retrieval_policy: sillage_governance::RetrievalSecurityPolicy,
    pub(crate) primary_generation: IndexGenerationId,
    pub(crate) dense_generation: Option<IndexGenerationId>,
    pub(crate) visual_embedding_provider:
        Option<Arc<dyn sillage_ports::VisualEmbeddingProvider + Send + Sync>>,
    pub(crate) visual_generation: Option<VisualGenerationCapability>,
    pub(crate) repository_code_index: Option<Arc<RepositoryCodeIndex>>,
    pub(crate) repository_execution_policy: RepositoryExecutionPolicy,
    pub(crate) hybrid_execution_policy: sillage_retrieval::HybridExecutionPolicy,
    pub(crate) visual_execution_policy: VisualExecutionPolicy,
    pub(crate) learned_sparse_execution_policy: sillage_retrieval::LearnedSparseExecutionPolicy,
    pub(crate) sparse_retriever: Option<Arc<dyn CandidateRetriever>>,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) scope_id: sillage_domain::ScopeId,
    pub(crate) fingerprint: RetrievalModelFingerprint,
    pub(crate) engine_cache: EngineCache,
    pub(crate) interactive_cache: InteractiveCache,
    pub(crate) source_manifest: Option<Arc<RwLock<InstanceManifest>>>,
    pub(crate) source_layout: Option<InstanceLayout>,
    pub(crate) interactive_search_workers: Arc<tokio::sync::Semaphore>,
    allowed_roots: Option<Arc<[PathBuf]>>,
}

pub(crate) use parts::SearchRuntimeParts;
pub(crate) use projection::reconcile_active_versions;

impl SearchRuntime {
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
        *self.interactive_cache.write() = None;
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
    ) -> Result<Arc<sillage_retrieval::RetrievalEngine>> {
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

    /// Rebuild only when a source-version event changes. Search access audits
    /// append events too, but cannot change the lexical source snapshot.
    pub(crate) fn interactive_snapshot(&self) -> Result<InteractiveSnapshot> {
        let revision = self.event_log.searchable_source_revision()?;
        let cached = self.interactive_cache.read().clone();
        if let Some(snapshot) = &cached
            && snapshot.revision == revision
        {
            return Ok(snapshot.clone());
        }
        let (events, sources, projection) = if self.repository_code_index.is_some() {
            // The code security resolver projects additional event families.
            let events = self.domain_events()?;
            let sources = Arc::new(sillage_domain::active_source_versions(&events));
            (events, sources, None)
        } else if let Some(previous) = cached
            .as_ref()
            .filter(|snapshot| snapshot.revision < revision)
            && let Some(projection) = previous.projection.as_deref()
        {
            let events = self
                .event_log
                .scan_searchable_source_events_between(previous.revision, revision)
                .map_err(|error| anyhow!("scan new source events for retrieval: {error}"))?;
            let mut projection = projection.clone();
            projection.apply(&events);
            let sources = projection.sources();
            (events, sources, Some(Arc::new(projection)))
        } else {
            let events = self
                .event_log
                .scan_searchable_source_events()
                .map_err(|error| anyhow!("scan source history for retrieval: {error}"))?;
            let mut projection = sillage_domain::SourceProjection::default();
            projection.apply(&events);
            let sources = projection.sources();
            (events, sources, Some(Arc::new(projection)))
        };
        let mut runtime = self.clone();
        runtime.graph_index = None;
        runtime.persist_learned_sparse_observations = false;
        let snapshot = InteractiveSnapshot {
            revision,
            engine: Arc::new(runtime.retrieval_engine_from_snapshot(&events, &sources)?),
            sources,
            projection,
            approved: Arc::new(RwLock::new(None)),
        };
        if self.event_log.searchable_source_revision()? == revision {
            *self.interactive_cache.write() = Some(snapshot.clone());
        }
        Ok(snapshot)
    }

    pub(crate) fn interactive_current_sources(&self) -> Result<(i64, Arc<ActiveSourceVersions>)> {
        let snapshot = self.interactive_snapshot()?;
        Ok((snapshot.revision, snapshot.sources))
    }
}
