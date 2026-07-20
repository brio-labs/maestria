use maestria_domain::{
    CorpusSnapshotId, EvidenceCoverage, IndexGenerationId, RetrievalModelFingerprint,
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace, SearchTraceFilter,
};
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
#[path = "learned_sparse_shadow.rs"]
mod learned_sparse_shadow;
pub use learned_sparse_shadow::{
    LearnedSparseShadowCandidate, LearnedSparseShadowLane, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowObservation, LearnedSparseShadowStore, LearnedSparseShadowStoreError,
};
#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

#[path = "planner.rs"]
mod planner;
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
    visual_reranker: bool,
    expander: Option<Arc<dyn ContextExpander>>,
    evaluator: Arc<dyn RetrievalEvaluator>,
    capabilities: maestria_governance::SearchCapabilities,
    hybrid_policy: crate::types::HybridExecutionPolicy,
    learned_sparse_execution_policy: crate::learned_sparse_policy::LearnedSparseExecutionPolicy,
    learned_sparse_shadow_store: learned_sparse_shadow::LearnedSparseShadowStore,
    repository_execution_policy: crate::repository_benchmark::RepositoryExecutionPolicy,
    visual_execution_policy: crate::visual_benchmark::VisualExecutionPolicy,
}

pub(super) use engine_capabilities::batch_is_eligible;

impl RetrievalEngine {
    fn validate_plan(&self, plan: &SearchPlan) -> RetrievalResult<()> {
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(plan.corpus_snapshot);
        let policy = maestria_governance::RetrievalSecurityPolicy::default();
        match maestria_governance::SearchPlanValidator::validate(plan, &capabilities, &policy) {
            Ok(()) => Ok(()),
            Err(maestria_governance::SearchPlanValidationError::IntentMismatch {
                declared: maestria_domain::SearchIntent::FactualLocal,
                classified,
            }) if classified != maestria_domain::SearchIntent::ExactLookup => {
                let mut fallback_plan = plan.clone();
                fallback_plan.original_query = "fallback local text retrieval".to_string();
                maestria_governance::SearchPlanValidator::validate(
                    &fallback_plan,
                    &capabilities,
                    &policy,
                )
                .map_err(RetrievalError::SearchPlan)
            }
            Err(error) => Err(RetrievalError::SearchPlan(error)),
        }
    }

    pub fn new(
        retrievers: Vec<Arc<dyn CandidateRetriever>>,
        evaluator: Arc<dyn RetrievalEvaluator>,
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
            hybrid_policy: crate::types::HybridExecutionPolicy::Shadow,
            learned_sparse_execution_policy:
                crate::learned_sparse_policy::LearnedSparseExecutionPolicy::Shadow,
            learned_sparse_shadow_store:
                learned_sparse_shadow::LearnedSparseShadowStore::default(),
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
    ) -> RetrievalResult<(
        SearchOutcome,
        Vec<maestria_domain::SearchTraceLane>,
        Option<maestria_domain::SearchTraceRerank>,
        maestria_domain::SearchTraceDiversity,
    )> {
        engine_evaluation::evaluate_batches(self, plan, query, batches, started).await
    }

    pub async fn search(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        if maestria_governance::contains_prompt_injection_risk(&plan.original_query) {
            return Ok(self.prompt_injection_outcome(plan));
        }
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

    pub(super) fn active_retrievers(&self, plan: &SearchPlan) -> Vec<Arc<dyn CandidateRetriever>> {
        let repository_specialized = self
            .repository_execution_policy
            .allows_specialized(&plan.original_query);
        let visual_enabled = self
            .visual_execution_policy
            .allows_visual(&plan.original_query);
        let sparse_enabled = self
            .learned_sparse_execution_policy
            .allows_sparse(&plan.original_query);
        self.retrievers
            .iter()
            .filter(|retriever| {
                let descriptor = retriever.descriptor();
                let descriptor_id = descriptor.id.to_ascii_lowercase();
                let is_code = descriptor.modality.eq_ignore_ascii_case("code")
                    || descriptor.modality.eq_ignore_ascii_case("rust")
                    || descriptor_id.contains("code_intel");
                crate::visual_benchmark::visual_lane_is_eligible(&descriptor, visual_enabled)
                    && crate::learned_sparse_policy::sparse_lane_is_eligible(
                        &descriptor,
                        sparse_enabled,
                    )
                    && (repository_specialized || !is_code)
            })
            .cloned()
            .collect()
    }

    fn learned_sparse_shadow_retrievers(
        &self,
        plan: &SearchPlan,
    ) -> Vec<Arc<dyn CandidateRetriever>> {
        if !self
            .learned_sparse_execution_policy
            .should_shadow(&plan.original_query)
        {
            return Vec::new();
        }
        self.retrievers
            .iter()
            .filter(|retriever| {
                crate::learned_sparse_policy::is_sparse_descriptor(&retriever.descriptor())
            })
            .cloned()
            .collect()
    }

    fn prompt_injection_outcome(&self, plan: &SearchPlan) -> SearchOutcome {
        let retriever_ids = self
            .retrievers
            .iter()
            .map(|retriever| retriever.descriptor().id.clone())
            .collect();
        let trace = SearchTrace::from_plan(
            plan,
            retriever_ids,
            &[],
            vec![SearchTraceFilter::PromptInjection],
            self.fusion.as_ref().map(|_| "configured".to_string()),
            Vec::new(),
            SearchStopReason::PolicyDenied,
        );
        SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: plan.fingerprint.clone(),
            index_generation: plan.index_generation,
            status: SearchStatus::QuarantinedForReview,
            evidence: Vec::new(),
            coverage: EvidenceCoverage {
                required_claims: vec![],
                required_subquestions: vec![],
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
                candidate_coverage_keys: vec![],
                percent_covered: 0,
                gaps_identified: vec![],
            },
            conflicts: Vec::new(),
        }
    }

    async fn search_internal(
        &self,
        plan: &SearchPlan,
        started: tokio::time::Instant,
    ) -> RetrievalResult<SearchOutcome> {
        let metadata = maestria_domain::SecurityMetadata {
            prompt_injection_risk: maestria_governance::contains_prompt_injection_risk(
                &plan.original_query,
            ),
            ..maestria_domain::SecurityMetadata::default()
        };
        let decision = maestria_governance::RetrievalSecurityPolicy::new()
            .require_read_allowed(true)
            .allow_unscoped_items(true)
            .evaluate(&metadata);
        if matches!(decision, maestria_governance::RetrievalDecision::Denied(_))
            && metadata.prompt_injection_risk
        {
            return Ok(self.prompt_injection_outcome(plan));
        }
        learned_sparse_shadow::spawn_learned_sparse_shadow(
            self.learned_sparse_shadow_retrievers(plan),
            plan.clone(),
            self.learned_sparse_shadow_store.clone(),
        );
        let active_retrievers = self.active_retrievers(plan);
        if active_retrievers.is_empty() {
            return Err(RetrievalError::Internal("No retrievers configured".into()));
        }
        let query = SearchQuery {
            q: plan.original_query.clone(),
            limit: plan.stop_conditions.max_results as usize,
            offset: 0,
        };
        let (batches, rewrites, web_requests_used, web_bytes_read) =
            engine_pipeline::collect_initial_batches(&active_retrievers, plan).await?;
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
