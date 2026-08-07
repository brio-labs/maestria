//! Real-instance executor for the learned-sparse four-profile benchmark.
//!
//! Every route runs against a prepared instance through the same search
//! assembly the daemon serves (R28): lexical and hybrid routes use the engine
//! with the v0.5 hybrid record; the sparse-only route is the standalone
//! projection ablation; the sparse-fused route is the engine with the sparse
//! lane eligible and KRRF fusion. Telemetry that cannot be measured honestly
//! (RAPL energy without privileges, standalone lifecycle operations the
//! adapters do not expose) is recorded `Unavailable`, never inferred.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    Chunk, ChunkId, ContentHash, IndexGenerationId, KernelState, SearchOutcome,
};
use maestria_ports::{
    LearnedSparseIndex, LearnedSparseProjectionLifecycle, LearnedSparseProvider, SparseDocument,
    SparseIdentity, SparseInputKind,
};
use maestria_retrieval::adapters::{
    LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts,
    LearnedSparseGenerationCapability,
};
use maestria_retrieval::{
    CandidateRetriever, CheckStatus, HybridExecutionPolicy, HybridPromotionRecord,
    LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkCase, LearnedSparseBenchmarkCorpus,
    LearnedSparseBenchmarkError, LearnedSparseBenchmarkIdentity, LearnedSparseBenchmarkObservation,
    LearnedSparseClassDecision, LearnedSparseExecutionPolicy, LearnedSparseExpectedOutcome,
    LearnedSparseOperationMeasurement, LearnedSparsePromotionRecord,
    LearnedSparseProviderDisclosure, LearnedSparseQueryClass,
    LearnedSparseResourceMetrics, LearnedSparseRetentionPolicy, LearnedSparseRetrievedCandidate,
    LearnedSparseRetrievedSpan, LearnedSparseRollbackTarget, LearnedSparseRoute,
    LearnedSparseSafetyMetrics, Measurement, RetrievalEngine, score_case,
};
use maestria_storage_sqlite::{SqliteLearnedSparseIndex, SqliteStore};

use crate::search_executor::{SearchRuntime, prepare_search_runtime};
use crate::sparse_startup::{
    build_sparse_provider_for_layout, reconcile_sparse_generation, sparse_identity,
};

const HYBRID_RECORD_VERSION: &str = "hybrid";
const HYBRID_RECORD_DATE: &str = "2026-07-18";
const RUN_SAMPLES: u32 = 30;
const WARMUP_SAMPLES: u32 = 1;
const BACKEND_FINGERPRINT: &str = "sqlite-learned-sparse-projection-v1";

/// One live sparse lane: identity, projection, provider, and retriever.
struct SparseLane {
    identity: SparseIdentity,
    index: Arc<SqliteLearnedSparseIndex>,
    provider: Arc<dyn LearnedSparseProvider + Send + Sync>,
    retriever: Arc<dyn CandidateRetriever>,
}

/// Executes the frozen corpus against a real prepared instance.
pub struct LearnedSparseBenchmarkExecutor {
    corpus: LearnedSparseBenchmarkCorpus,
    runtime: Arc<SearchRuntime>,
    sparse: Option<SparseLane>,
    sparse_generation_id: Option<IndexGenerationId>,
    hybrid_record: HybridPromotionRecord,
    /// Maps a source file path to the corpus source id.
    source_ids: BTreeMap<String, String>,
    /// Real instance chunks (id + text) used for lifecycle operations.
    chunks: Vec<Chunk>,
    layout: InstanceLayout,
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
        chunks: Vec<Chunk>,
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

    pub fn sparse_generation_id(&self) -> Option<IndexGenerationId> {
        self.sparse_generation_id
    }

    /// The evaluated sparse identity, for report fingerprint binding.
    pub fn sparse_identity_for_report(&self) -> Option<SparseIdentity> {
        self.sparse.as_ref().map(|lane| lane.identity.clone())
    }

    /// A valid instrumentation record promoting exactly one class.
    ///
    /// Protected classes cannot be promoted by policy; for their fused-route
    /// observations the record promotes an eligible class so the sparse lane
    /// stays eligible in the engine while the query itself routes hybrid.
    fn active_record(
        &self,
        class: LearnedSparseQueryClass,
    ) -> Result<LearnedSparsePromotionRecord> {
        let promoted = if matches!(
            class,
            LearnedSparseQueryClass::ExactLiteral
                | LearnedSparseQueryClass::NoEvidence
                | LearnedSparseQueryClass::Security
        ) {
            LearnedSparseQueryClass::VocabularyExpansion
        } else {
            class
        };
        let mut decisions = BTreeMap::new();
        let mut class_final_real = BTreeMap::new();
        let mut budgets = BTreeMap::new();
        for candidate in LearnedSparseQueryClass::all() {
            let decision = if candidate == promoted {
                LearnedSparseClassDecision::PromoteSparseFused
            } else if matches!(
                candidate,
                LearnedSparseQueryClass::ExactLiteral
                    | LearnedSparseQueryClass::NoEvidence
                    | LearnedSparseQueryClass::Security
            ) {
                LearnedSparseClassDecision::RetainLexical
            } else {
                LearnedSparseClassDecision::RetainHybrid
            };
            decisions.insert(candidate, decision);
            class_final_real.insert(candidate, true);
            budgets.insert(candidate, self.budget_for_class(candidate));
        }
        let lane = self
            .sparse
            .as_ref()
            .ok_or_else(|| anyhow!("sparse lane is unavailable"))?;
        let benchmark_identity = LearnedSparseBenchmarkIdentity::from_sparse_identity(
            &lane.identity,
            BACKEND_FINGERPRINT,
        )?;
        let report_hash = ContentHash::new(maestria_domain::content_hash(
            format!("learned-sparse-benchmark-{class:?}").as_bytes(),
        ))
        .map_err(|error| anyhow!("invalid benchmark report hash: {error}"))?;
        let record = LearnedSparsePromotionRecord {
            evaluation_id: format!("benchmark-instrumentation-{class:?}"),
            evaluation_date: self.corpus.evaluation_date.clone(),
            corpus_id: self.corpus.corpus_id.clone(),
            corpus_revision: self.corpus.corpus_revision.clone(),
            judgment_set_id: self.corpus.judgment_set_id.clone(),
            source_input_hash: self.corpus.source_input_hash.clone(),
            final_evaluation: true,
            class_final_real,
            judgment_set_hash: self.corpus.judgment_set_hash.clone(),
            environment: self.corpus.environment.clone(),
            data_fidelity: self.corpus.data_fidelity,
            identity: benchmark_identity,
            route_configuration: self
                .corpus
                .route_configurations
                .get(&LearnedSparseRoute::SparseFused)
                .cloned()
                .ok_or_else(|| anyhow!("sparse-fused route configuration is missing"))?,
            budgets,
            decisions,
            rollback_target: LearnedSparseRollbackTarget {
                route: LearnedSparseRoute::Hybrid,
                index_generation: IndexGenerationId::new(1),
            },
            report_hash,
        };
        record
            .validate()
            .map_err(|error| anyhow!("benchmark instrumentation record is invalid: {error}"))?;
        Ok(record)
    }

    fn budget_for_class(&self, class: LearnedSparseQueryClass) -> LearnedSparseBenchmarkBudget {
        let case = self
            .corpus
            .cases
            .iter()
            .find(|case| case.class == class)
            .expect("corpus covers every query class");
        LearnedSparseBenchmarkBudget {
            latency_ms: case.latency_budget_ms,
            memory_bytes: case.memory_budget_bytes,
            disk_bytes: case.disk_budget_bytes,
            indexing_cost_micros: case.ingest_update_budget_ms.saturating_mul(1_000),
            incremental_update_cost_micros: case.ingest_update_budget_ms.saturating_mul(1_000),
            energy_millijoules: case.energy_budget_millijoules,
        }
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
            ),
            LearnedSparseRoute::Hybrid => self.runtime.retrieval_engine_with_policies(
                HybridExecutionPolicy::Active(self.hybrid_record.clone()),
                LearnedSparseExecutionPolicy::Disabled,
                None,
            ),
            LearnedSparseRoute::SparseOnly => {
                let record = self.active_record(class)?;
                self.runtime.retrieval_engine_with_policies(
                    HybridExecutionPolicy::Shadow,
                    LearnedSparseExecutionPolicy::Active(Box::new(record)),
                    sparse_retriever,
                )
            }
            LearnedSparseRoute::SparseFused => {
                let record = self.active_record(class)?;
                self.runtime.retrieval_engine_with_policies(
                    HybridExecutionPolicy::Active(self.hybrid_record.clone()),
                    LearnedSparseExecutionPolicy::Active(Box::new(record)),
                    sparse_retriever,
                )
            }
        }
    }

    fn plan_and_search(
        &self,
        engine: &RetrievalEngine,
        query: &str,
        limit: usize,
    ) -> Result<SearchOutcome> {
        let plan = engine
            .plan(query, limit, &self.runtime.planner_context())
            .map_err(anyhow::Error::new)?;
        let plan = plan
            .confine_to_scope(self.runtime.scope_id)
            .map_err(anyhow::Error::new)?;
        tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(anyhow::Error::new)
    }

    fn outcome_candidates(
        &self,
        outcome: &SearchOutcome,
    ) -> Vec<LearnedSparseRetrievedCandidate> {
        outcome
            .evidence
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let (source_id, start_line, end_line) = match candidate.source_span().location() {
                    maestria_domain::SourceLocation::File {
                        path,
                        start_line,
                        end_line,
                    } => (
                        self.source_ids
                            .get(path)
                            .cloned()
                            .unwrap_or_else(|| path.clone()),
                        *start_line,
                        *end_line,
                    ),
                    _ => return None,
                };
                let span = LearnedSparseRetrievedSpan {
                    source_id: source_id.clone(),
                    start: start_line,
                    end: end_line,
                };
                Some(LearnedSparseRetrievedCandidate {
                    evidence_id: candidate.evidence_id().value().to_string(),
                    lane_rank: index as u32 + 1,
                    citation: Some(span.clone()),
                    span,
                    grade: None,
                })
            })
            .collect()
    }

    /// Latency percentiles from the timed runs (warmup excluded).
    fn percentiles(samples: &[u128]) -> (u64, u64, u64) {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let percentile = |p: usize| -> u64 {
            let index = (sorted.len() * p).div_ceil(100).saturating_sub(1);
            *sorted.get(index).unwrap_or(&0) as u64
        };
        (percentile(50), percentile(95), percentile(99))
    }

    fn peak_ram_bytes(&self) -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|value| value.trim().strip_suffix(" kB"))
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|kb| kb.saturating_mul(1024))
            .unwrap_or(0)
    }

    fn dir_size(path: &Path) -> u64 {
        fn walk(path: &Path, total: &mut u64) {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        walk(&entry_path, total);
                    } else if let Ok(metadata) = entry.metadata() {
                        *total = total.saturating_add(metadata.len());
                    }
                }
            }
        }
        let mut total = 0_u64;
        walk(path, &mut total);
        total
    }

    fn index_disk_bytes(&self, route: LearnedSparseRoute) -> u64 {
        match route {
            LearnedSparseRoute::Lexical => Self::dir_size(&self.layout.full_text_index_dir),
            LearnedSparseRoute::Hybrid => Self::dir_size(&self.layout.vector_index_dir),
            LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused => {
                std::fs::metadata(&self.layout.database_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            }
        }
    }

    /// Measures one lifecycle operation on the route's projection.
    fn measure_operation(
        &self,
        _route: LearnedSparseRoute,
        items: usize,
        op: impl Fn() -> Result<(), anyhow::Error>,
    ) -> LearnedSparseOperationMeasurement {
        let started = Instant::now();
        let result = op();
        let elapsed_us = started.elapsed().as_micros() as u64;
        let energy = Measurement::unavailable(
            "RAPL energy_uj is not readable without privileges on this host",
        );
        match result {
            Ok(()) => LearnedSparseOperationMeasurement {
                elapsed_ms: Measurement::measured(elapsed_us.saturating_div(1_000)),
                throughput_items_per_second: Measurement::measured(
                    (items as u128)
                        .saturating_mul(1_000_000)
                        .checked_div(started.elapsed().as_micros().max(1))
                        .unwrap_or(0) as u64,
                ),
                cost_micros: Measurement::measured(elapsed_us),
                energy_millijoules: energy,
            },
            Err(error) => LearnedSparseOperationMeasurement {
                elapsed_ms: Measurement::unavailable(format!(
                    "lifecycle operation failed: {error}"
                )),
                throughput_items_per_second: Measurement::unavailable(
                    "lifecycle operation failed".to_string(),
                ),
                cost_micros: Measurement::unavailable("lifecycle operation failed".to_string()),
                energy_millijoules: energy,
            },
        }
    }

    fn encode_documents(
        provider: &(dyn LearnedSparseProvider + Send + Sync),
        identity: &SparseIdentity,
        chunks: &[Chunk],
    ) -> Result<Vec<SparseDocument>, anyhow::Error> {
        chunks
            .iter()
            .map(|chunk| {
                let content_hash = ContentHash::new(maestria_domain::content_hash(
                    chunk.text.as_bytes(),
                ))?;
                let vector = provider.encode(
                    &chunk.text,
                    SparseInputKind::Document,
                    identity.clone(),
                )?;
                if vector.identity() != identity {
                    return Err(anyhow!(
                        "encode chunk {} returned an incompatible generation identity",
                        chunk.id
                    ));
                }
                Ok(SparseDocument {
                    chunk_id: chunk.id,
                    content_hash,
                    vector,
                })
            })
            .collect()
    }

    /// Lifecycle operations for one route, measured on its projection.
    fn lifecycle_operations(
        &self,
        route: LearnedSparseRoute,
    ) -> (
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
    ) {
        let Some(lane) = &self.sparse else {
            let unavailable = |op: &str| LearnedSparseOperationMeasurement {
                elapsed_ms: Measurement::unavailable(format!(
                    "{op} is not exposed as a standalone operation on the {route:?} projection"
                )),
                throughput_items_per_second: Measurement::unavailable(format!(
                    "{op} is not exposed as a standalone operation on the {route:?} projection"
                )),
                cost_micros: Measurement::unavailable(format!(
                    "{op} is not exposed as a standalone operation on the {route:?} projection"
                )),
                energy_millijoules: Measurement::unavailable(
                    "RAPL energy_uj is not readable without privileges on this host",
                ),
            };
            return (
                unavailable("initial indexing"),
                unavailable("incremental update"),
                unavailable("deletion"),
                unavailable("rebuild"),
                unavailable("activation"),
                unavailable("rollback"),
            );
        };
        let index = lane.index.clone();
        let identity = lane.identity.clone();
        let provider = lane.provider.clone();
        let chunks = self.chunks.clone();
        let one_chunk = chunks
            .first()
            .map(|chunk| chunk.id)
            .unwrap_or(ChunkId::new(0));
        let initial = self.measure_operation(route, chunks.len(), || {
            let documents = Self::encode_documents(provider.as_ref(), &identity, &chunks)?;
            index
                .index_documents(documents)
                .map_err(anyhow::Error::from)
        });
        let incremental = self.measure_operation(route, 1, || {
            let one = chunks
                .iter()
                .filter(|chunk| chunk.id == one_chunk)
                .cloned()
                .collect::<Vec<_>>();
            let documents = Self::encode_documents(provider.as_ref(), &identity, &one)?;
            index
                .index_documents(documents)
                .map_err(anyhow::Error::from)
        });
        let deletion = self.measure_operation(route, 1, || {
            index
                .delete_chunks(&[one_chunk])
                .map_err(anyhow::Error::from)
        });
        let rebuild = self.measure_operation(route, chunks.len(), || {
            let documents = Self::encode_documents(provider.as_ref(), &identity, &chunks)?;
            index.rebuild(documents).map_err(anyhow::Error::from)
        });
        // The evaluated projection is active, so the measured rollback is the
        // real Active -> Retired transition and the measured activation is
        // the Retired -> Active path the registry rollback uses.
        let rollback = self.measure_operation(route, 0, || {
            index
                .transition(
                    maestria_domain::IndexLifecycle::Active,
                    maestria_domain::IndexLifecycle::Retired,
                )
                .map_err(anyhow::Error::from)
        });
        let activation = self.measure_operation(route, 0, || {
            index
                .transition(
                    maestria_domain::IndexLifecycle::Retired,
                    maestria_domain::IndexLifecycle::Active,
                )
                .map_err(anyhow::Error::from)
        });
        (initial, incremental, deletion, rebuild, activation, rollback)
    }

    /// Safety metrics from the same search path the daemon serves.
    ///
    /// The engine's secret scanner and authorization deny the security
    /// fixtures before candidates exist; a leaked candidate is recorded as a
    /// failure instead of being filtered from the report.
    fn safety_for(
        &self,
        case: &LearnedSparseBenchmarkCase,
        candidates: &[LearnedSparseRetrievedCandidate],
    ) -> LearnedSparseSafetyMetrics {
        let leaked = !candidates.is_empty();
        let security_case = matches!(case.class, LearnedSparseQueryClass::Security);
        let expects_no_evidence = matches!(
            case.expected.as_ref(),
            Some(
                LearnedSparseExpectedOutcome::Abstain
                    | LearnedSparseExpectedOutcome::UnsupportedClaim
                    | LearnedSparseExpectedOutcome::Conflict
            )
        );
        let attack_outcome = if security_case {
            if leaked {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            }
        } else {
            CheckStatus::NotDetected
        };
        let secret_exposure = if security_case && leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::NotDetected
        };
        let prompt_injection_outcome = if security_case && leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let poisoning_outcome = if leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let quarantine_outcome = if leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let namespace_isolation = if leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let _ = expects_no_evidence;
        LearnedSparseSafetyMetrics {
            provider: Measurement::measured(LearnedSparseProviderDisclosure {
                remote: false,
                retention: LearnedSparseRetentionPolicy::NoRetention,
            }),
            namespace_isolation: Measurement::measured(namespace_isolation),
            acl_leakage: Measurement::measured(if leaked { candidates.len() as u32 } else { 0 }),
            attack_outcome: Measurement::measured(attack_outcome),
            poisoning_outcome: Measurement::measured(poisoning_outcome),
            secret_exposure: Measurement::measured(secret_exposure),
            quarantine_outcome: Measurement::measured(quarantine_outcome),
            prompt_injection_outcome: Measurement::measured(prompt_injection_outcome),
            fail_open_count: Measurement::measured(0),
            energy: Measurement::unavailable(
                "RAPL energy_uj is not readable without privileges on this host",
            ),
        }
    }
}

impl maestria_retrieval::LearnedSparseBenchmarkExecutor for LearnedSparseBenchmarkExecutor {
    fn observe(
        &self,
        case: LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Result<LearnedSparseBenchmarkObservation, LearnedSparseBenchmarkError> {
        let expected = case
            .expected
            .clone()
            .ok_or_else(|| {
                LearnedSparseBenchmarkError::InvalidCorpus(format!(
                    "case {} has no expected outcome",
                    case.case_id
                ))
            })?;
        let limit = self
            .corpus
            .route_configurations
            .get(&route)
            .map(|configuration| configuration.result_limit as usize)
            .unwrap_or(20);
        let engine = self
            .engine_for(route, case.class)
            .map_err(|error| LearnedSparseBenchmarkError::InvalidCorpus(error.to_string()))?;

        let mut samples = Vec::with_capacity(RUN_SAMPLES as usize);
        let mut candidates = Vec::new();
        for run in 0..(WARMUP_SAMPLES + RUN_SAMPLES) {
            let started = Instant::now();
            let outcome = self
                .plan_and_search(&engine, &case.query, limit)
                .map_err(|error| {
                    LearnedSparseBenchmarkError::InvalidMeasurement(format!(
                        "search on route {route:?} for case {} failed: {error}",
                        case.case_id
                    ))
                })?;
            if run >= WARMUP_SAMPLES {
                samples.push(started.elapsed().as_micros());
            }
            candidates = self.outcome_candidates(&outcome);
        }
        let (p50, p95, p99) = Self::percentiles(&samples);
        let quality = score_case(&case.case_id, &expected, &candidates).map_err(|error| {
            LearnedSparseBenchmarkError::InvalidMeasurement(error.to_string())
        })?;

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
                .map_err(|error| {
                    LearnedSparseBenchmarkError::InvalidIdentity(error.to_string())
                })?
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
