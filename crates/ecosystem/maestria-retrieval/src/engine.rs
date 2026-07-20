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
#[path = "engine_evaluation.rs"]
mod engine_evaluation;
#[path = "engine_pipeline.rs"]
mod engine_pipeline;
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
    repository_execution_policy: crate::repository_benchmark::RepositoryExecutionPolicy,
    visual_execution_policy: crate::visual_benchmark::VisualExecutionPolicy,
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

fn batch_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    hybrid_policy: &crate::types::HybridExecutionPolicy,
    repository_specialized: bool,
) -> bool {
    let id = descriptor.id.to_ascii_lowercase();
    let is_dense = id.contains("dense") || id.contains("vector") || id.contains("semantic");
    let is_code = descriptor.modality.eq_ignore_ascii_case("code")
        || descriptor.modality.eq_ignore_ascii_case("rust")
        || id.contains("code_intel");
    let hybrid_allowed = match hybrid_policy {
        crate::types::HybridExecutionPolicy::Shadow => !is_dense,
        crate::types::HybridExecutionPolicy::Active(_) => true,
    };
    hybrid_allowed && (repository_specialized || !is_code)
}

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
        let capabilities = capabilities_from_retrievers(&retrievers);
        Self {
            retrievers,
            fusion: None,
            reranker: None,
            visual_reranker: false,
            expander: None,
            evaluator,
            capabilities,
            hybrid_policy: crate::types::HybridExecutionPolicy::Shadow,
            repository_execution_policy:
                crate::repository_benchmark::RepositoryExecutionPolicy::Shadow,
            visual_execution_policy: crate::visual_benchmark::VisualExecutionPolicy::Shadow,
        }
    }

    pub fn with_hybrid_policy(mut self, policy: crate::types::HybridExecutionPolicy) -> Self {
        self.hybrid_policy = policy;
        self
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
        self.retrievers
            .iter()
            .filter(|retriever| {
                let descriptor = retriever.descriptor();
                let descriptor_id = descriptor.id.to_ascii_lowercase();
                let is_code = descriptor.modality.eq_ignore_ascii_case("code")
                    || descriptor.modality.eq_ignore_ascii_case("rust")
                    || descriptor_id.contains("code_intel");
                crate::visual_benchmark::visual_lane_is_eligible(&descriptor, visual_enabled)
                    && (repository_specialized || !is_code)
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
