//! Per-route search execution for the four-profile benchmark.

use anyhow::{Result, anyhow};
use maestria_domain::{SearchOutcome, SearchPlan};
use maestria_ports::{SearchQuery, SparseIdentity};
use maestria_retrieval::{
    HybridExecutionPolicy, LearnedSparseBenchmarkCase, LearnedSparseBenchmarkError,
    LearnedSparseExecutionPolicy, LearnedSparseQueryClass, LearnedSparseRoute, MonotonicInstant,
    RetrievalEngine,
};

use super::LearnedSparseBenchmarkExecutor;

impl LearnedSparseBenchmarkExecutor {
    pub(super) fn engine_for(
        &self,
        route: LearnedSparseRoute,
        class: LearnedSparseQueryClass,
    ) -> Result<RetrievalEngine> {
        let sparse_retriever = self.sparse.as_ref().map(|lane| lane.retriever.clone());
        let mut engine = match route {
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
        }?;
        if matches!(
            route,
            LearnedSparseRoute::Hybrid | LearnedSparseRoute::SparseFused
        ) && let Some(fusion) = &self.fusion
        {
            engine = engine.with_fusion(fusion.clone());
        }
        Ok(engine)
    }
}

impl LearnedSparseBenchmarkExecutor {
    /// The evaluated sparse identity, for report fingerprint binding.
    pub fn sparse_identity_for_report(&self) -> Option<SparseIdentity> {
        self.sparse.as_ref().map(|lane| lane.identity.clone())
    }

    pub(super) fn plan_for(
        &self,
        engine: &RetrievalEngine,
        route: LearnedSparseRoute,
        query: &str,
        limit: usize,
    ) -> Result<SearchPlan> {
        let mut context = self.runtime.planner_context();
        if route == LearnedSparseRoute::SparseOnly {
            // The sparse-only ablation serves exclusively from the sparse
            // generation, so its plan must target that generation.
            context.primary_generation = self
                .sparse_generation_id
                .ok_or_else(|| anyhow!("sparse generation is unavailable"))?;
        }
        let plan = engine
            .plan(query, limit, &context)
            .map_err(anyhow::Error::new)?;
        plan.confine_to_scope(self.runtime.scope_id)
            .map_err(anyhow::Error::new)
    }

    fn plan_and_search(
        &self,
        engine: &RetrievalEngine,
        plan: &SearchPlan,
    ) -> Result<SearchOutcome> {
        // The engine is async; the daemon runs it on a blocking worker so the
        // benchmark can run both inside and outside an existing runtime.
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(anyhow::Error::new)?;
                    runtime
                        .block_on(engine.search(plan))
                        .map_err(anyhow::Error::new)
                })
                .join()
                .map_err(|_| anyhow!("benchmark search worker panicked"))?
        })
    }

    /// The sparse-only ablation: the projection's own retriever through the
    /// same authorization path the engine applies, scored standalone.
    ///
    /// Returns `None` when the engine refuses to plan the query on this
    /// instance; the route then abstains honestly.
    fn sparse_only_candidates(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<maestria_retrieval::LearnedSparseRetrievedCandidate>>> {
        let lane = self
            .sparse
            .as_ref()
            .ok_or_else(|| anyhow!("sparse lane is unavailable"))?;
        let generation_id = self
            .sparse_generation_id
            .ok_or_else(|| anyhow!("sparse generation is unavailable"))?;
        let route_configuration = self
            .corpus
            .route_configurations
            .get(&LearnedSparseRoute::SparseOnly)
            .cloned()
            .ok_or_else(|| anyhow!("sparse-only route configuration is missing"))?;
        let engine = self.engine_for(
            LearnedSparseRoute::SparseOnly,
            maestria_retrieval::LearnedSparseQueryClass::VocabularyExpansion,
        )?;
        let plan = match self.plan_for(&engine, LearnedSparseRoute::SparseOnly, query, limit) {
            Ok(plan) => plan,
            Err(error) => {
                tracing::warn!("sparse-only plan refused, recording abstention: {error}");
                return Ok(None);
            }
        };
        let authorization = self
            .runtime
            .retrieval_policy
            .authorization_context(plan.scope())
            .map_err(anyhow::Error::new)?;
        let request = maestria_retrieval::types::CandidateRequest {
            plan: std::sync::Arc::new(plan.clone()),
            query: SearchQuery {
                q: query.to_string(),
                limit,
                offset: 0,
                execution_budget: route_configuration.budget,
            },
            execution_budget: route_configuration.budget,
            expected_generation: generation_id,
            authorization,
            source_filter: None,
        };
        let batch = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(anyhow::Error::new)?;
                    runtime
                        .block_on(lane.retriever.retrieve(request))
                        .map_err(anyhow::Error::new)
                })
                .join()
                .map_err(|_| anyhow!("benchmark sparse-only worker panicked"))?
        })?;
        Ok(Some(self.candidates_from(batch.candidates)))
    }

    pub(super) fn candidates_from(
        &self,
        candidates: Vec<maestria_domain::EvidenceCandidate>,
    ) -> Vec<maestria_retrieval::LearnedSparseRetrievedCandidate> {
        candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let (source_id, start_line, end_line) = match candidate.source_span().location() {
                    maestria_domain::SourceLocation::File {
                        path,
                        start_line,
                        end_line,
                    } => {
                        let source_id = match self.source_ids.get(path) {
                            Some(source_id) => source_id.clone(),
                            None => path.clone(),
                        };
                        (source_id, *start_line, *end_line)
                    }
                    _ => return None,
                };
                let span = maestria_retrieval::LearnedSparseRetrievedSpan {
                    source_id: source_id.clone(),
                    start: start_line,
                    end: end_line,
                };
                Some(maestria_retrieval::LearnedSparseRetrievedCandidate {
                    evidence_id: candidate.evidence_id().value().to_string(),
                    lane_rank: index as u32 + 1,
                    citation: Some(span.clone()),
                    span,
                    grade: None,
                })
            })
            .collect()
    }

    fn outcome_candidates(
        &self,
        outcome: &SearchOutcome,
    ) -> Vec<maestria_retrieval::LearnedSparseRetrievedCandidate> {
        self.candidates_from(outcome.evidence.clone())
    }

    /// Latency percentiles from the timed runs (warmup excluded).
    pub(super) fn percentiles(samples: &[u128]) -> (u64, u64, u64) {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let percentile = |p: usize| -> u64 {
            let index = (sorted.len() * p).div_ceil(100).saturating_sub(1);
            match sorted.get(index) {
                Some(value) => *value as u64,
                None => 0,
            }
        };
        (percentile(50), percentile(95), percentile(99))
    }

    /// One warmup plus `RUN_SAMPLES` timed retrievals for a case and route.
    pub(super) fn timed_retrievals(
        &self,
        case: &LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
        engine: &RetrievalEngine,
        limit: usize,
    ) -> Result<(
        Vec<maestria_retrieval::LearnedSparseRetrievedCandidate>,
        Vec<u128>,
    )> {
        let mut samples = Vec::with_capacity(super::RUN_SAMPLES as usize);
        let mut candidates = Vec::new();
        for run in 0..(super::WARMUP_SAMPLES + super::RUN_SAMPLES) {
            let started = MonotonicInstant::now();
            if route == LearnedSparseRoute::SparseOnly {
                let sparse = self
                    .sparse_only_candidates(&case.query, limit)
                    .map_err(|error| {
                        LearnedSparseBenchmarkError::InvalidMeasurement(format!(
                            "sparse-only retrieval on route {route:?} for case {} failed: {error}",
                            case.case_id
                        ))
                    })?;
                candidates = sparse.into_iter().flatten().collect();
            } else {
                candidates = match self.plan_for(engine, route, &case.query, limit) {
                    Ok(plan) => {
                        let outcome = self.plan_and_search(engine, &plan).map_err(|error| {
                            LearnedSparseBenchmarkError::InvalidMeasurement(format!(
                                "search on route {route:?} for case {} failed: {error}",
                                case.case_id
                            ))
                        })?;
                        self.outcome_candidates(&outcome)
                    }
                    Err(error) => {
                        // A plan the engine refuses (unsupported intent or
                        // modality on this instance) is an honest route
                        // abstention: no evidence is produced or fabricated.
                        // Recorded once per observation, not per run.
                        if run == super::WARMUP_SAMPLES {
                            tracing::warn!(
                                "case {} route {route:?}: plan refused, recording abstention: \
                                 {error}",
                                case.case_id
                            );
                        }
                        Vec::new()
                    }
                };
            }
            if run >= super::WARMUP_SAMPLES {
                // The latency metrics are millisecond-valued; the monotonic
                // samples are converted here so the percentile computation
                // stays in one unit.
                samples.push(started.elapsed().as_millis());
            }
        }
        Ok((candidates, samples))
    }
}
