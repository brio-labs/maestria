use maestria_domain::{SearchOutcome, SearchPlan};
use std::sync::Arc;

use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};
use crate::types::{CandidateSourceFilter, RetrievalError, RetrievalResult};
#[path = "engine_adaptive.rs"]
mod engine_adaptive;
#[path = "engine_capabilities.rs"]
mod engine_capabilities;
#[path = "engine_config.rs"]
mod engine_config;
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
pub(crate) use engine_capabilities::batch_is_eligible;
/// Allocates the execution budget for one retrieval lane.
pub use engine_pipeline::lane_budget;
pub(crate) use engine_pipeline::partition_allowance;
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

impl RetrievalEngine {
    /// Execute the search plan and return the outcome.
    ///
    /// # Cancellation
    /// The latency budget is enforced at the pipeline's preemption points:
    /// the adaptive iteration loop and the bounded reranker check the
    /// elapsed deadline between blocking operations, and provider calls
    /// are bounded by the transport agent timeout.
    pub fn search(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        let authorization = self
            .security_policy
            .authorization_context(plan.scope())
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval authorization denied: {error:?}"))
            })?;
        self.search_pre_authorized(plan, authorization)
    }

    /// Executes with a caller-composed authorization context. This is the
    /// federation boundary: all retrieval lanes consume this context before
    /// scoring and must not reconstruct policy from the engine configuration.
    ///
    /// # Cancellation
    /// The latency budget is enforced at the pipeline's preemption points;
    /// provider calls are bounded by the transport agent timeout.
    pub fn search_pre_authorized(
        &self,
        plan: &SearchPlan,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> RetrievalResult<SearchOutcome> {
        self.search_pre_authorized_with_filter(plan, authorization, None)
    }

    /// Executes an authorized search restricted to the selected artifact set.
    ///
    /// # Cancellation
    /// The latency budget is enforced at the pipeline's preemption points;
    /// provider calls are bounded by the transport agent timeout.
    pub fn search_pre_authorized_selected(
        &self,
        plan: &SearchPlan,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        source_filter: CandidateSourceFilter,
    ) -> RetrievalResult<SearchOutcome> {
        self.search_pre_authorized_with_filter(plan, authorization, Some(source_filter))
    }

    fn search_pre_authorized_with_filter(
        &self,
        plan: &SearchPlan,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<CandidateSourceFilter>,
    ) -> RetrievalResult<SearchOutcome> {
        self.validate_plan(plan)?;
        if maestria_governance::contains_prompt_injection_risk(plan.original_query()) {
            return self.prompt_injection_outcome(plan, source_filter.as_ref());
        }
        self.search_internal(
            plan,
            crate::MonotonicInstant::now(),
            authorization,
            source_filter,
        )
    }

    fn search_internal(
        &self,
        plan: &SearchPlan,
        started: crate::MonotonicInstant,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        source_filter: Option<CandidateSourceFilter>,
    ) -> RetrievalResult<SearchOutcome> {
        let shadow_task = learned_sparse_shadow::spawn_learned_sparse_shadow(
            self.learned_sparse_shadow_retrievers(plan),
            plan.clone(),
            authorization.clone(),
            source_filter.clone(),
            self.learned_sparse_shadow_store.clone(),
        );
        let active_result: RetrievalResult<SearchOutcome> = {
            let active_retrievers = self.active_retrievers(plan);
            if active_retrievers.is_empty() {
                return Err(RetrievalError::Internal("No retrievers configured".into()));
            }
            let query = engine_pipeline::search_query_for_plan(plan, plan.original_query())?;
            let (batches, rewrites, web_requests_used, mut execution_usage) =
                engine_pipeline::collect_initial_batches(
                    &active_retrievers,
                    plan,
                    &authorization,
                    source_filter.as_ref(),
                )?;
            let (outcome, lanes, rerank_trace, diversity_trace) =
                engine_evaluation::evaluate_batches(engine_evaluation::EvaluationRequest {
                    engine: self,
                    plan,
                    query: &query,
                    batches: &batches,
                    started,
                    execution_usage: &mut execution_usage,
                    authorization: &authorization,
                    source_filter: source_filter.as_ref(),
                })?;
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
                source_filter.as_ref(),
                &mut state,
                started,
            )?;
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
                    source_selection_digest: source_filter
                        .as_ref()
                        .map(CandidateSourceFilter::digest),
                    rerank_trace: state.rerank_trace,
                    diversity_trace: Some(state.diversity_trace),
                    rewrites: state.rewrites.trace_records(),
                    explicit_stop_reason,
                },
            )?;
            outcome.verify_compatibility(plan)?;
            Ok(outcome)
        };
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
