use maestria_domain::{SearchOutcome, SearchPlan};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use std::time::Duration;

use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};
use crate::types::{RetrievalError, RetrievalResult};

#[path = "engine_adaptive.rs"]
mod engine_adaptive;
#[path = "engine_capabilities.rs"]
mod engine_capabilities;
#[path = "engine_evaluation.rs"]
mod engine_evaluation;
#[path = "engine_pipeline.rs"]
mod engine_pipeline;
#[path = "engine_policy.rs"]
mod engine_policy;
#[path = "learned_sparse_shadow.rs"]
mod learned_sparse_shadow;
pub use learned_sparse_shadow::{
    LearnedSparseShadowCandidate, LearnedSparseShadowLane, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowObservation, LearnedSparseShadowRoute, LearnedSparseShadowStore,
    LearnedSparseShadowStoreError,
};
#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

#[path = "planner.rs"]
mod planner;
/// Allocates the execution budget for one retrieval lane.
pub use engine_pipeline::lane_budget;
/// Reconciles an evaluator status with the diversity selector status.
pub use engine_pipeline::reconcile_status;
pub use planner::SearchPlannerContext;
/// Builds a rewrite session for the plan's query with the plan budgets.
pub use planner::rewrite_session;

#[path = "engine_trace.rs"]
mod engine_trace;
/// Governed search-trace construction helpers re-exported by the engine façade.
pub use engine_trace::{
    EnsureTraceOptions, applied_security_filters, ensure_trace, security_policy_fingerprint,
};

pub struct RetrievalEngine {
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    fusion: Option<Arc<dyn RankFusion>>,
    reranker: Option<Arc<dyn CandidateReranker>>,
    visual_reranker: bool,
    expander: Option<Arc<dyn ContextExpander>>,
    evaluator: Arc<dyn RetrievalEvaluator>,
    capabilities: maestria_governance::SearchCapabilities,
    security_policy: maestria_governance::RetrievalSecurityPolicy,
    hybrid_policy: crate::types::HybridExecutionPolicy,
    learned_sparse_execution_policy: crate::learned_sparse_policy::LearnedSparseExecutionPolicy,
    learned_sparse_shadow_store: learned_sparse_shadow::LearnedSparseShadowStore,
    repository_execution_policy: crate::repository_benchmark::RepositoryExecutionPolicy,
    visual_execution_policy: crate::visual_benchmark::VisualExecutionPolicy,
}

pub(super) use engine_capabilities::batch_is_eligible;

impl RetrievalEngine {
    pub fn new(
        retrievers: Vec<Arc<dyn CandidateRetriever>>,
        evaluator: Arc<dyn RetrievalEvaluator>,
        security_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self {
        let capabilities = engine_capabilities::capabilities_from_retrievers(&retrievers);
        Self {
            retrievers,
            fusion: None,
            reranker: None,
            visual_reranker: false,
            expander: None,
            evaluator,
            capabilities,
            security_policy,
            hybrid_policy: crate::types::HybridExecutionPolicy::Shadow,
            learned_sparse_execution_policy:
                crate::learned_sparse_policy::LearnedSparseExecutionPolicy::Shadow,
            learned_sparse_shadow_store: learned_sparse_shadow::LearnedSparseShadowStore::default(),
            repository_execution_policy:
                crate::repository_benchmark::RepositoryExecutionPolicy::Shadow,
            visual_execution_policy: crate::visual_benchmark::VisualExecutionPolicy::Shadow,
        }
    }

    pub fn with_hybrid_policy(mut self, policy: crate::types::HybridExecutionPolicy) -> Self {
        self.hybrid_policy = policy;
        self
    }

    pub fn with_learned_sparse_execution_policy(
        mut self,
        policy: crate::learned_sparse_policy::LearnedSparseExecutionPolicy,
    ) -> Self {
        self.learned_sparse_execution_policy = policy;
        self
    }

    pub fn with_learned_sparse_shadow_store(
        mut self,
        store: learned_sparse_shadow::LearnedSparseShadowStore,
    ) -> Self {
        self.learned_sparse_shadow_store = store;
        self
    }

    pub fn with_learned_sparse_observation_repository(
        mut self,
        repository: Arc<dyn maestria_ports::LearnedSparseObservationRepository>,
    ) -> Self {
        self.learned_sparse_shadow_store =
            self.learned_sparse_shadow_store.with_repository(repository);
        self
    }

    pub fn learned_sparse_shadow_store(&self) -> learned_sparse_shadow::LearnedSparseShadowStore {
        self.learned_sparse_shadow_store.clone()
    }

    pub fn with_repository_execution_policy(
        mut self,
        policy: crate::repository_benchmark::RepositoryExecutionPolicy,
    ) -> Self {
        self.repository_execution_policy = policy;
        self
    }

    pub fn with_visual_execution_policy(
        mut self,
        policy: crate::visual_benchmark::VisualExecutionPolicy,
    ) -> Self {
        self.visual_execution_policy = policy;
        self
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

    pub fn with_visual_reranker(mut self, reranker: Arc<dyn CandidateReranker>) -> Self {
        self.reranker = Some(reranker);
        self.visual_reranker = true;
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
        execution_usage: &mut maestria_domain::SearchExecutionUsage,
        authorization: &maestria_governance::RetrievalAuthorizationContext,
    ) -> RetrievalResult<(
        SearchOutcome,
        Vec<maestria_domain::SearchTraceLane>,
        Option<maestria_domain::SearchTraceRerank>,
        maestria_domain::SearchTraceDiversity,
    )> {
        engine_evaluation::evaluate_batches(
            self,
            plan,
            query,
            batches,
            started,
            execution_usage,
            authorization,
        )
        .await
    }

    /// Execute the search plan and return the outcome.
    ///
    /// # Cancellation
    /// Dropping the future aborts the search. When `timeout_ms` is greater than zero, the search
    /// is also aborted if the latency budget is exceeded.
    pub async fn search(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        let authorization = self
            .security_policy
            .authorization_context(plan.scope())
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval authorization denied: {error:?}"))
            })?;
        self.search_pre_authorized(plan, authorization).await
    }

    /// Executes with a caller-composed authorization context. This is the
    /// federation boundary: all retrieval lanes consume this context before
    /// scoring and must not reconstruct policy from the engine configuration.
    ///
    /// # Cancellation
    /// Dropping the future aborts the active search and its owned shadow task. When
    /// `timeout_ms` is greater than zero, the search is also aborted if the latency budget
    /// is exceeded.
    pub async fn search_pre_authorized(
        &self,
        plan: &SearchPlan,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> RetrievalResult<SearchOutcome> {
        self.validate_plan(plan)?;
        if maestria_governance::contains_prompt_injection_risk(plan.original_query()) {
            return self.prompt_injection_outcome(plan);
        }
        let timeout_ms = plan.budgets().max_latency_ms() as u64;
        let started = tokio::time::Instant::now();
        let search = self.search_internal(plan, started, authorization);
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
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> RetrievalResult<SearchOutcome> {
        let shadow_task = learned_sparse_shadow::spawn_learned_sparse_shadow(
            self.learned_sparse_shadow_retrievers(plan),
            plan.clone(),
            authorization.clone(),
            self.learned_sparse_shadow_store.clone(),
        );
        let active_result = async {
            let active_retrievers = self.active_retrievers(plan);
            if active_retrievers.is_empty() {
                return Err(RetrievalError::Internal("No retrievers configured".into()));
            }
            let query = SearchQuery {
                q: plan.original_query().to_string(),
                limit: plan.stop_conditions().max_results as usize,
                offset: 0,
                execution_budget: plan.execution_budget()?,
            };
            let (batches, rewrites, web_requests_used, mut execution_usage) =
                engine_pipeline::collect_initial_batches(&active_retrievers, plan, &authorization)
                    .await?;
            let (outcome, lanes, rerank_trace, diversity_trace) = self
                .evaluate_batches(
                    plan,
                    &query,
                    &batches,
                    started,
                    &mut execution_usage,
                    &authorization,
                )
                .await?;
            let mut state = engine_adaptive::AdaptiveSearchState {
                batches,
                rewrites,
                web_requests_used,
                execution_usage,
                outcome,
                lanes,
                rerank_trace,
                diversity_trace,
            };
            let explicit_stop_reason = engine_adaptive::iterate_until_stop(
                self,
                plan,
                &query,
                &authorization,
                &mut state,
                started,
            )
            .await?;
            let expansion_enabled = plan
                .stages()
                .contains(&maestria_domain::SearchStage::Filtering);
            let mut trace_policy = self.security_policy.clone();
            trace_policy.required_scope_id = None;
            trace_policy.instance_scope_ids = authorization.effective_scopes().cloned();
            let outcome = ensure_trace(
                plan,
                state.outcome,
                state.lanes,
                EnsureTraceOptions {
                    security_policy: trace_policy,
                    fusion_enabled: self.fusion.is_some(),
                    expansion_enabled,
                    rerank_trace: state.rerank_trace,
                    diversity_trace: Some(state.diversity_trace),
                    rewrites: state.rewrites.trace_records(),
                    explicit_stop_reason,
                },
            )?;
            outcome.verify_compatibility(plan)?;
            Ok(outcome)
        }
        .await;
        match active_result {
            Ok(outcome) => {
                if let Some(shadow_task) = shadow_task {
                    shadow_task.release();
                }
                Ok(outcome)
            }
            Err(error) => Err(error),
        }
    }
}
