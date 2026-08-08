//! Real-instance executor for the learned-sparse four-profile benchmark.
//!
//! Every route runs against a prepared instance through the same search
//! assembly the daemon serves (R28): lexical and hybrid routes use the engine
//! with the v0.5 hybrid record; the sparse-only route is the standalone
//! projection ablation; the sparse-fused route is the engine with the sparse
//! lane eligible and KRRF fusion. Telemetry that cannot be measured honestly
//! (RAPL energy without privileges, standalone lifecycle operations the
//! adapters do not expose) is recorded `Unavailable`, never inferred.
//!
//! Responsibility map:
//! - `record`: benchmark instrumentation promotion records.
//! - `lifecycle`: lifecycle operations measured on the route's projection.
//! - `safety`: safety metrics from the served search path.
//! - `search`: per-route search execution and candidate mapping.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::KernelState;
use maestria_ports::{LearnedSparseIndex, LearnedSparseProvider, SparseIdentity};
use maestria_retrieval::adapters::{
    LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts,
    LearnedSparseGenerationCapability,
};
use maestria_retrieval::{
    CandidateRetriever, HybridExecutionPolicy, HybridPromotionRecord, LearnedSparseBenchmarkCase,
    LearnedSparseBenchmarkCorpus, LearnedSparseBenchmarkError, LearnedSparseBenchmarkIdentity,
    LearnedSparseBenchmarkObservation, LearnedSparseExecutionPolicy, LearnedSparseQueryClass,
    LearnedSparseResourceMetrics, LearnedSparseRoute, Measurement, RetrievalEngine, score_case,
};
use maestria_storage_sqlite::{SqliteLearnedSparseIndex, SqliteStore};

#[path = "learned_sparse_benchmark_executor/lifecycle.rs"]
mod lifecycle;
#[path = "learned_sparse_benchmark_executor/record.rs"]
mod record;
#[path = "learned_sparse_benchmark_executor/safety.rs"]
mod safety;
#[path = "learned_sparse_benchmark_executor/search.rs"]
mod search;

use crate::search_executor::{SearchRuntime, prepare_search_runtime};
use crate::sparse_startup::{
    build_sparse_provider_for_layout, reconcile_sparse_generation,
    reconcile_sparse_projection_for_layout, sparse_identity,
};

const HYBRID_RECORD_VERSION: &str = "hybrid";
const HYBRID_RECORD_DATE: &str = "2026-07-18";
pub(super) const RUN_SAMPLES: u32 = 30;
pub(super) const WARMUP_SAMPLES: u32 = 1;
pub(super) const BACKEND_FINGERPRINT: &str = "sqlite-learned-sparse-projection-v1";

/// One live sparse lane: identity, projection, provider, and retriever.
pub(super) struct SparseLane {
    pub(super) identity: SparseIdentity,
    pub(super) index: Arc<SqliteLearnedSparseIndex>,
    pub(super) provider: Arc<dyn LearnedSparseProvider + Send + Sync>,
    pub(super) retriever: Arc<dyn CandidateRetriever>,
}

/// Executes the frozen corpus against a real prepared instance.
pub struct LearnedSparseBenchmarkExecutor {
    pub(super) corpus: LearnedSparseBenchmarkCorpus,
    pub(super) runtime: Arc<SearchRuntime>,
    pub(super) sparse: Option<SparseLane>,
    pub(super) sparse_generation_id: Option<maestria_domain::IndexGenerationId>,
    hybrid_record: HybridPromotionRecord,
    /// Maps a source file path to the corpus source id.
    pub(super) source_ids: BTreeMap<String, String>,
    /// Real instance chunks (id + text) used for lifecycle operations.
    pub(super) chunks: Vec<maestria_domain::Chunk>,
    pub(super) layout: InstanceLayout,
}

impl LearnedSparseBenchmarkExecutor {
    /// Prepares the instance for evaluation: reconciles the sparse
    /// generation and projection, opens the search runtime, and builds the
    /// sparse lane when the profile is enabled.
    pub fn prepare(
        layout: &InstanceLayout,
        state: &mut KernelState,
        manifest: &InstanceManifest,
        corpus: LearnedSparseBenchmarkCorpus,
        source_ids: BTreeMap<String, String>,
        chunks: Vec<maestria_domain::Chunk>,
    ) -> Result<Self> {
        corpus.validate()?;
        let runtime = prepare_search_runtime(
            layout,
            state,
            manifest,
            maestria_governance::RetrievalSecurityPolicy::default(),
        )?;
        let (sparse, sparse_generation_id) = if manifest
            .sparse
            .as_ref()
            .is_some_and(|config| config.enabled)
        {
            let generation_id = reconcile_sparse_generation(layout, state, manifest)?;
            // Populates the projection from the replayed chunks and advances
            // its lifecycle to Active so the retriever can serve.
            reconcile_sparse_projection_for_layout(layout, state, manifest)?;
            let identity = sparse_identity(state, manifest, generation_id)?;
            let provider = build_sparse_provider_for_layout(manifest, state)?
                .ok_or_else(|| anyhow!("sparse provider is not configured"))?;
            let store = SqliteStore::open(&layout.database_path)
                .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
            let index = Arc::new(
                SqliteLearnedSparseIndex::new(Arc::new(store), identity.clone())
                    .map_err(|error| anyhow!("open sparse projection: {error}"))?,
            );
            let capability = LearnedSparseGenerationCapability::activate(
                &state.index_generations,
                identity.clone(),
            )
            .map_err(|error| anyhow!("activate sparse generation: {error}"))?;
            let retriever = LearnedSparseChunkRetriever::new(
                LearnedSparseChunkRetrieverParts {
                    index: index.clone() as Arc<dyn LearnedSparseIndex + Send + Sync>,
                    artifacts: runtime.artifacts.clone(),
                    chunks: runtime.chunks.clone(),
                    evidence: runtime.evidence.clone(),
                    blobs: runtime.blobs.clone(),
                    provider: provider.clone(),
                },
                capability,
            )
            .map_err(|error| anyhow!("build sparse retriever: {error}"))?;
            (
                Some(SparseLane {
                    identity,
                    index,
                    provider,
                    retriever: Arc::new(retriever),
                }),
                Some(generation_id),
            )
        } else {
            (None, None)
        };
        let hybrid_record = HybridPromotionRecord::new(
            HYBRID_RECORD_VERSION.to_string(),
            HYBRID_RECORD_DATE.to_string(),
        )
        .ok_or_else(|| anyhow!("hybrid promotion record is unavailable"))?;
        Ok(Self {
            corpus,
            runtime,
            sparse,
            sparse_generation_id,
            hybrid_record,
            source_ids,
            chunks,
            layout: layout.clone(),
        })
    }

    pub fn sparse_generation_id(&self) -> Option<maestria_domain::IndexGenerationId> {
        self.sparse_generation_id
    }

    /// The evaluated sparse identity, for report fingerprint binding.
    pub fn sparse_identity_for_report(&self) -> Option<SparseIdentity> {
        self.sparse.as_ref().map(|lane| lane.identity.clone())
    }

    fn engine_for(
        &self,
        route: LearnedSparseRoute,
        class: LearnedSparseQueryClass,
    ) -> Result<RetrievalEngine> {
        let sparse_retriever = self.sparse.as_ref().map(|lane| lane.retriever.clone());
        match route {
            LearnedSparseRoute::Lexical => self.runtime.retrieval_engine_with_policies(
                HybridExecutionPolicy::Shadow,
                LearnedSparseExecutionPolicy::Disabled,
                None,
                true,
            ),
            LearnedSparseRoute::Hybrid => self.runtime.retrieval_engine_with_policies(
                HybridExecutionPolicy::Active(self.hybrid_record.clone()),
                LearnedSparseExecutionPolicy::Disabled,
                None,
                true,
            ),
            LearnedSparseRoute::SparseOnly => {
                let record = self.active_record(class)?;
                self.runtime.retrieval_engine_with_policies(
                    HybridExecutionPolicy::Shadow,
                    LearnedSparseExecutionPolicy::Active(Box::new(record)),
                    sparse_retriever,
                    false,
                )
            }
            LearnedSparseRoute::SparseFused => {
                let record = self.active_record(class)?;
                self.runtime.retrieval_engine_with_policies(
                    HybridExecutionPolicy::Active(self.hybrid_record.clone()),
                    LearnedSparseExecutionPolicy::Active(Box::new(record)),
                    sparse_retriever,
                    true,
                )
            }
        }
    }
}

impl maestria_retrieval::LearnedSparseBenchmarkExecutor for LearnedSparseBenchmarkExecutor {
    fn observe(
        &self,
        case: LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Result<LearnedSparseBenchmarkObservation, LearnedSparseBenchmarkError> {
        let expected = case.expected.clone().ok_or_else(|| {
            LearnedSparseBenchmarkError::InvalidCorpus(format!(
                "case {} has no expected outcome",
                case.case_id
            ))
        })?;
        let configuration = self
            .corpus
            .route_configurations
            .get(&route)
            .ok_or_else(|| {
                LearnedSparseBenchmarkError::InvalidCorpus(format!(
                    "route {route:?} configuration is missing"
                ))
            })?;
        let limit = configuration.result_limit as usize;
        let engine = self
            .engine_for(route, case.class)
            .map_err(|error| LearnedSparseBenchmarkError::InvalidCorpus(error.to_string()))?;
        let (candidates, samples) = self
            .timed_retrievals(&case, route, &engine, limit)
            .map_err(|error| LearnedSparseBenchmarkError::InvalidMeasurement(error.to_string()))?;
        let (p50, p95, p99) = Self::percentiles(&samples);
        let quality = score_case(&case.case_id, &expected, &candidates)
            .map_err(|error| LearnedSparseBenchmarkError::InvalidMeasurement(error.to_string()))?;

        let (initial_indexing, incremental_update, deletion, rebuild, activation, rollback) =
            self.lifecycle_operations(route);

        let safety = self.safety_for(&case, &candidates);

        Ok(LearnedSparseBenchmarkObservation {
            schema_version: 2,
            corpus_id: self.corpus.corpus_id.clone(),
            corpus_revision: self.corpus.corpus_revision.clone(),
            judgment_set_id: self.corpus.judgment_set_id.clone(),
            evaluation_date: self.corpus.evaluation_date.clone(),
            case_id: case.case_id,
            route,
            identity: self
                .sparse
                .as_ref()
                .map(|lane| {
                    LearnedSparseBenchmarkIdentity::from_sparse_identity(
                        &lane.identity,
                        BACKEND_FINGERPRINT,
                    )
                })
                .transpose()
                .map_err(|error| LearnedSparseBenchmarkError::InvalidIdentity(error.to_string()))?
                .ok_or_else(|| {
                    LearnedSparseBenchmarkError::InvalidIdentity(
                        "sparse identity is unavailable for the observation".to_string(),
                    )
                })?,
            route_configuration: self
                .corpus
                .route_configurations
                .get(&route)
                .cloned()
                .ok_or_else(|| {
                    LearnedSparseBenchmarkError::InvalidCorpus(format!(
                        "route {route:?} configuration is missing"
                    ))
                })?,
            quality,
            resources: LearnedSparseResourceMetrics {
                p50_latency_ms: Measurement::measured(p50),
                p95_latency_ms: Measurement::measured(p95),
                p99_latency_ms: Measurement::measured(p99),
                peak_ram_bytes: Measurement::measured(self.peak_ram_bytes()),
                index_disk_bytes: Measurement::measured(self.index_disk_bytes(route)),
                initial_indexing,
                incremental_update,
                deletion,
                rebuild,
                activation,
                rollback,
            },
            safety,
        })
    }
}
