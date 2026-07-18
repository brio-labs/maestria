use maestria_domain::{
    CorpusSnapshotId, IndexGenerationId, RetrievalModelFingerprint, SearchOutcome, SearchPlan,
};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use std::time::Duration;

use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};
use crate::types::{RankedCandidate, RerankRequest, RetrievalError, RetrievalResult};

#[path = "engine_adaptive.rs"]
mod engine_adaptive;
#[path = "engine_pipeline.rs"]
mod engine_pipeline;

pub(super) use engine_pipeline::reconcile_status;

pub(crate) fn rewrite_session(plan: &SearchPlan) -> crate::rewrite::QueryRewriteSession {
    let mut session = crate::rewrite::QueryRewriteSession::with_limits(
        &plan.original_query,
        plan.budgets.max_tokens() as usize,
        plan.budgets.max_latency_ms(),
        plan.budgets.max_queries(),
    );
    session.expand_deterministic();
    session
}

#[path = "engine_trace.rs"]
mod engine_trace;
pub(super) use engine_trace::{EnsureTraceOptions, ensure_trace};

/// Runtime inputs used to build a deterministic search plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPlannerContext {
    pub corpus_snapshot: CorpusSnapshotId,
    pub primary_generation: IndexGenerationId,
    pub fingerprint: RetrievalModelFingerprint,
}

pub struct RetrievalEngine {
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    fusion: Option<Arc<dyn RankFusion>>,
    reranker: Option<Arc<dyn CandidateReranker>>,
    expander: Option<Arc<dyn ContextExpander>>,
    evaluator: Arc<dyn RetrievalEvaluator>,
    capabilities: maestria_governance::SearchCapabilities,
    hybrid_policy: crate::types::HybridExecutionPolicy,
}

fn capabilities_from_retrievers(
    retrievers: &[Arc<dyn CandidateRetriever>],
) -> maestria_governance::SearchCapabilities {
    use maestria_domain::{
        CorpusSnapshotId, IndexGenerationId, Modality, SearchIntent, SearchStage,
    };

    let primary_generation = retrievers
        .iter()
        .map(|retriever| retriever.descriptor())
        .find(|descriptor| !descriptor.modality.eq_ignore_ascii_case("dense"))
        .map_or(IndexGenerationId::new(1), |descriptor| {
            descriptor.generation
        });
    let mut capabilities = maestria_governance::SearchCapabilities::new()
        .with_intent(SearchIntent::ExactLookup)
        .with_intent(SearchIntent::FactualLocal)
        .with_stage(SearchStage::InitialRetrieval)
        .with_snapshot(CorpusSnapshotId::new(1))
        .with_generation(primary_generation)
        .allow_global_scope()
        .max_scope_ids(u32::MAX)
        .max_budgets(1_000, 30_000, 8, 3, 0)
        .with_security_filters();
    let mut known_modality = false;
    for retriever in retrievers {
        match retriever
            .descriptor()
            .modality
            .to_ascii_lowercase()
            .as_str()
        {
            "text" | "lexical" => {
                capabilities = capabilities.with_modality(Modality::Text);
                known_modality = true;
            }
            "code" | "rust" => {
                capabilities = capabilities
                    .with_modality(Modality::Code)
                    .with_intent(SearchIntent::RepositoryCode);
                known_modality = true;
            }
            "image" => {
                capabilities = capabilities
                    .with_modality(Modality::Image)
                    .with_intent(SearchIntent::VisualDocument);
                known_modality = true;
            }
            "pdf" => {
                capabilities = capabilities.with_modality(Modality::Pdf);
                known_modality = true;
            }
            "table" => {
                capabilities = capabilities.with_modality(Modality::Table);
                known_modality = true;
            }
            "web" => {
                capabilities = capabilities
                    .with_modality(Modality::Web)
                    .with_intent(SearchIntent::CurrentWeb)
                    .enable_web()
                    .support_realtime()
                    .max_budgets(1_000, 30_000, 8, 3, 1);
                known_modality = true;
            }
            "vector" | "dense" | "semantic" => {
                capabilities = capabilities
                    .with_modality(Modality::Text)
                    .with_intent(SearchIntent::SemanticDiscovery);
                known_modality = true;
            }
            _ => {}
        }
    }
    if !known_modality {
        capabilities = capabilities.with_modality(Modality::Text);
    }
    capabilities
}

impl RetrievalEngine {
    fn validate_plan(&self, plan: &SearchPlan) -> RetrievalResult<()> {
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(plan.corpus_snapshot)
            .with_generation(plan.index_generation);
        maestria_governance::SearchPlanValidator::validate(
            plan,
            &capabilities,
            &maestria_governance::RetrievalSecurityPolicy::default(),
        )
        .map_err(RetrievalError::SearchPlan)
    }

    pub fn new(
        retrievers: Vec<Arc<dyn CandidateRetriever>>,
        evaluator: Arc<dyn RetrievalEvaluator>,
    ) -> Self {
        let capabilities = capabilities_from_retrievers(&retrievers);
        Self {
            retrievers,
            fusion: None,
            reranker: None,
            expander: None,
            evaluator,
            capabilities,
            hybrid_policy: crate::types::HybridExecutionPolicy::Shadow,
        }
    }

    pub fn with_hybrid_policy(mut self, policy: crate::types::HybridExecutionPolicy) -> Self {
        self.hybrid_policy = policy;
        self
    }

    /// Build a schema-valid plan whose capabilities are derived from installed lanes.
    pub fn plan(
        &self,
        query: impl Into<String>,
        limit: usize,
        context: &SearchPlannerContext,
    ) -> RetrievalResult<SearchPlan> {
        let original_query = query.into();
        let intent = maestria_domain::SearchIntent::classify(&original_query);
        let modality = match intent {
            maestria_domain::SearchIntent::RepositoryCode => maestria_domain::Modality::Code,
            maestria_domain::SearchIntent::VisualDocument => maestria_domain::Modality::Image,
            maestria_domain::SearchIntent::CurrentWeb => maestria_domain::Modality::Web,
            _ => maestria_domain::Modality::Text,
        };
        let (web_requests, bytes, concurrency) =
            if intent == maestria_domain::SearchIntent::CurrentWeb {
                (3, 1_000_000, 3)
            } else {
                (0, 0, 1)
            };
        let max_stages = if self.expander.is_some() { 2 } else { 1 };
        let budgets = maestria_domain::SearchBudget::with_resource_limits(
            1_000,
            30_000,
            8,
            max_stages,
            web_requests,
            bytes,
            concurrency,
        )
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        Ok(SearchPlan {
            query_id: maestria_domain::QueryId::new(1),
            original_query,
            intent,
            scope: maestria_domain::CorpusScope::Global,
            corpus_snapshot: context.corpus_snapshot,
            index_generation: context.primary_generation,
            freshness: if intent == maestria_domain::SearchIntent::CurrentWeb {
                maestria_domain::FreshnessRequirement::Realtime
            } else {
                maestria_domain::FreshnessRequirement::Any
            },
            modalities: maestria_domain::ModalitySet::new(vec![modality]),
            stages: if self.expander.is_some() {
                vec![
                    maestria_domain::SearchStage::InitialRetrieval,
                    maestria_domain::SearchStage::Filtering,
                ]
            } else {
                vec![maestria_domain::SearchStage::InitialRetrieval]
            },
            budgets,
            stop_conditions: maestria_domain::StopConditions {
                max_results: match u32::try_from(limit) {
                    Ok(value) => value.max(1),
                    Err(_) => u32::MAX,
                },
                min_score_threshold: 0,
            },
            evidence_requirements: maestria_domain::EvidenceRequirements {
                require_primary_sources: false,
                minimum_corroboration: 1,
                required_claims: Vec::new(),
                required_subquestions: Vec::new(),
                minimum_sources: 0,
                minimum_documents: 0,
                minimum_sections: 0,
            },
            fingerprint: context.fingerprint.clone(),
        })
    }

    pub fn with_capabilities(
        mut self,
        capabilities: maestria_governance::SearchCapabilities,
    ) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_fusion(mut self, fusion: Arc<dyn RankFusion>) -> Self {
        self.fusion = Some(fusion);
        self
    }

    pub fn with_reranker(mut self, reranker: Arc<dyn CandidateReranker>) -> Self {
        self.reranker = Some(reranker);
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Reranking);
        self
    }

    pub fn with_expander(mut self, expander: Arc<dyn ContextExpander>) -> Self {
        self.expander = Some(expander);
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Filtering);
        self
    }
    pub(super) async fn evaluate_batches(
        &self,
        plan: &SearchPlan,
        query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
        started: tokio::time::Instant,
    ) -> RetrievalResult<(
        SearchOutcome,
        Vec<maestria_domain::SearchTraceLane>,
        Option<maestria_domain::SearchTraceRerank>,
        maestria_domain::SearchTraceDiversity,
    )> {
        let lanes = engine_pipeline::trace_lanes(batches);
        let fusion_batches: Vec<_> = match self.hybrid_policy {
            crate::types::HybridExecutionPolicy::Shadow => batches
                .iter()
                .filter(|batch| {
                    let id = batch.descriptor.id.to_ascii_lowercase();
                    !id.contains("dense") && !id.contains("vector") && !id.contains("semantic")
                })
                .cloned()
                .collect(),
            crate::types::HybridExecutionPolicy::Active(_) => batches.to_vec(),
        };
        let mut ranked = if let Some(fusion) = &self.fusion {
            fusion
                .fuse(query, &fusion_batches)?
                .into_iter()
                .enumerate()
                .map(|(rank, fused)| RankedCandidate {
                    candidate: fused.candidate,
                    rank,
                })
                .collect()
        } else {
            batches
                .iter()
                .filter(|batch| {
                    matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded)
                })
                .flat_map(|batch| batch.candidates.iter().cloned())
                .enumerate()
                .map(|(rank, candidate)| RankedCandidate { candidate, rank })
                .collect()
        };
        let mut rerank_trace = None;
        if plan
            .stages
            .contains(&maestria_domain::SearchStage::Reranking)
            && let Some(reranker) = &self.reranker
        {
            let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let remaining_ms = u64::from(plan.budgets.max_latency_ms())
                .saturating_sub(elapsed_ms)
                .min(u64::from(u32::MAX)) as u32;
            let rerank_res = reranker
                .rerank(RerankRequest {
                    plan: plan.clone(),
                    candidates: ranked,
                    max_latency_ms: remaining_ms,
                })
                .await?;
            ranked = rerank_res.candidates;
            rerank_trace = Some(rerank_res.trace);
        }
        let expansion_enabled = plan
            .stages
            .contains(&maestria_domain::SearchStage::Filtering);
        let configured_expander = expansion_enabled.then(|| self.expander.clone()).flatten();
        let initial_diversity = crate::diversity::select_candidates(&ranked, plan);
        let (mut raw_outcome, final_diversity) = engine_pipeline::run_diversity_stage(
            plan,
            initial_diversity,
            &configured_expander,
            &self.evaluator,
        )
        .await?;
        raw_outcome.status = reconcile_status(&raw_outcome.status, &final_diversity.status);
        raw_outcome.coverage = final_diversity.coverage.clone();
        Ok((raw_outcome, lanes, rerank_trace, final_diversity.trace))
    }
    pub async fn search(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        self.validate_plan(plan)?;
        let timeout_ms = plan.budgets.max_latency_ms() as u64;
        let started = tokio::time::Instant::now();
        let search = self.search_internal(plan, started);
        if timeout_ms > 0 {
            tokio::time::timeout(Duration::from_millis(timeout_ms), search)
                .await
                .map_err(|_| RetrievalError::Timeout)?
        } else {
            search.await
        }
    }

    async fn search_internal(
        &self,
        plan: &SearchPlan,
        started: tokio::time::Instant,
    ) -> RetrievalResult<SearchOutcome> {
        if self.retrievers.is_empty() {
            return Err(RetrievalError::Internal("No retrievers configured".into()));
        }
        let query = SearchQuery {
            q: plan.original_query.clone(),
            limit: plan.stop_conditions.max_results as usize,
            offset: 0,
        };
        let (batches, rewrites, web_requests_used, web_bytes_read) =
            engine_pipeline::collect_initial_batches(&self.retrievers, plan).await?;
        let (outcome, lanes, rerank_trace, diversity_trace) = self
            .evaluate_batches(plan, &query, &batches, started)
            .await?;
        let mut state = engine_adaptive::AdaptiveSearchState {
            batches,
            rewrites,
            web_requests_used,
            web_bytes_read,
            outcome,
            lanes,
            rerank_trace,
            diversity_trace,
        };
        let explicit_stop_reason =
            engine_adaptive::iterate_until_stop(self, plan, &query, &mut state, started).await?;
        let expansion_enabled = plan
            .stages
            .contains(&maestria_domain::SearchStage::Filtering);
        let outcome = ensure_trace(
            plan,
            state.outcome,
            state.lanes,
            EnsureTraceOptions {
                fusion_enabled: self.fusion.is_some(),
                expansion_enabled,
                rerank_trace: state.rerank_trace,
                diversity_trace: Some(state.diversity_trace),
                rewrites: state.rewrites.trace_records(),
                explicit_stop_reason,
            },
        );
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
    }
}
