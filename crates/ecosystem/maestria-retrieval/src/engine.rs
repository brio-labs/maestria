use maestria_domain::{
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace, SearchTraceExpansion,
};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use std::time::Duration;

use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};
use crate::types::{RankedCandidate, RerankRequest, RetrievalError, RetrievalResult};

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

pub(crate) struct EnsureTraceOptions {
    pub(crate) fusion_enabled: bool,
    pub(crate) expansion_enabled: bool,
    pub(crate) rerank_trace: Option<maestria_domain::SearchTraceRerank>,
    pub(crate) diversity_trace: Option<maestria_domain::SearchTraceDiversity>,
    pub(crate) rewrites: Vec<maestria_domain::SearchTraceRewrite>,
}

pub(super) fn ensure_trace(
    plan: &SearchPlan,
    mut outcome: SearchOutcome,
    lanes: Vec<maestria_domain::SearchTraceLane>,
    options: EnsureTraceOptions,
) -> SearchOutcome {
    let EnsureTraceOptions {
        fusion_enabled,
        expansion_enabled,
        rerank_trace,
        diversity_trace,
        rewrites,
    } = options;
    let trace_is_valid = outcome.trace_data.as_ref().is_some_and(|trace| {
        outcome.trace == trace.deterministic_id()
            && trace.matches_plan(plan)
            && trace.retrievers
                == lanes
                    .iter()
                    .map(|l| l.retriever_id.clone())
                    .collect::<Vec<_>>()
            && trace.lanes == lanes
            && trace.fusion.is_some() == fusion_enabled
            && trace.rerank == rerank_trace
            && trace.diversity == diversity_trace
            && trace.expansions.len() == usize::from(expansion_enabled)
            && trace.rewrites == rewrites
            && trace.matches_evidence(&outcome.evidence)
    });
    if trace_is_valid {
        return outcome;
    }
    let stop_reason = match &outcome.status {
        SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
            SearchStopReason::PolicyDenied
        }
        SearchStatus::Abstained => SearchStopReason::Abstained,
        SearchStatus::NoEvidenceFound => SearchStopReason::NoEvidence,
        SearchStatus::SourcesConflict
        | SearchStatus::EvidenceIncomplete
        | SearchStatus::StaleEvidenceOnly => SearchStopReason::RequirementsUnmet,
        _ => diversity_trace.as_ref().map_or_else(
            || {
                if outcome.evidence.len() >= plan.stop_conditions.max_results as usize {
                    SearchStopReason::ResultsLimit
                } else {
                    SearchStopReason::EvidenceComplete
                }
            },
            |trace| trace.stop_reason.clone(),
        ),
    };
    let expansions = expansion_enabled
        .then_some(SearchTraceExpansion {
            strategy: "configured".to_string(),
            added_candidates: None,
        })
        .into_iter()
        .collect();
    let mut trace = SearchTrace::from_plan(
        plan,
        lanes.iter().map(|l| l.retriever_id.clone()).collect(),
        &outcome.evidence,
        Vec::new(),
        fusion_enabled.then_some("configured".to_string()),
        expansions,
        stop_reason,
    )
    .with_lanes(lanes)
    .with_gaps_and_conflicts(
        outcome.coverage.gaps_identified.clone(),
        outcome
            .conflicts
            .iter()
            .map(|conflict| conflict.id)
            .collect(),
    );
    trace.rewrites = rewrites;
    trace.rerank = rerank_trace;
    trace.diversity = diversity_trace;
    outcome.trace = trace.deterministic_id();
    outcome.trace_data = Some(Box::new(trace));
    outcome
}

pub struct RetrievalEngine {
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    fusion: Option<Arc<dyn RankFusion>>,
    reranker: Option<Arc<dyn CandidateReranker>>,
    expander: Option<Arc<dyn ContextExpander>>,
    evaluator: Arc<dyn RetrievalEvaluator>,
    capabilities: maestria_governance::SearchCapabilities,
}

fn capabilities_from_retrievers(
    retrievers: &[Arc<dyn CandidateRetriever>],
) -> maestria_governance::SearchCapabilities {
    use maestria_domain::{
        CorpusSnapshotId, IndexGenerationId, Modality, SearchIntent, SearchStage,
    };

    let mut capabilities = maestria_governance::SearchCapabilities::new()
        .with_intent(SearchIntent::ExactLookup)
        .with_intent(SearchIntent::FactualLocal)
        .with_stage(SearchStage::InitialRetrieval)
        .with_snapshot(CorpusSnapshotId::new(1))
        .with_generation(IndexGenerationId::new(1))
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
                capabilities = capabilities.with_modality(Modality::Image);
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
        maestria_governance::SearchPlanValidator::validate(
            plan,
            &self.capabilities,
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
        }
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
        let session = rewrite_session(plan);
        let mut batches = Vec::new();
        for rewrite in session
            .records()
            .iter()
            .filter(|record| record.stage == crate::rewrite::StageRole::InitialRetrieval)
        {
            let rewrite_query = SearchQuery {
                q: rewrite.query.clone(),
                limit: query.limit,
                offset: 0,
            };
            batches.extend(
                engine_pipeline::collect_batches(&self.retrievers, plan, &rewrite_query).await?,
            );
        }
        let lanes = engine_pipeline::trace_lanes(&batches);
        let mut ranked = if let Some(fusion) = &self.fusion {
            fusion
                .fuse(&query, &batches)?
                .into_iter()
                .enumerate()
                .map(|(rank, fused)| RankedCandidate {
                    candidate: fused.candidate,
                    rank,
                })
                .collect()
        } else {
            batches
                .into_iter()
                .filter(|batch| {
                    matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded)
                })
                .flat_map(|batch| batch.candidates)
                .enumerate()
                .map(|(rank, candidate)| RankedCandidate { candidate, rank })
                .collect()
        };

        let mut rerank_trace = None;
        let reranking_enabled = plan
            .stages
            .contains(&maestria_domain::SearchStage::Reranking);
        if reranking_enabled && let Some(reranker) = &self.reranker {
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
        raw_outcome.coverage = final_diversity.coverage;
        let outcome = ensure_trace(
            plan,
            raw_outcome,
            lanes,
            EnsureTraceOptions {
                fusion_enabled: self.fusion.is_some(),
                expansion_enabled,
                rerank_trace,
                diversity_trace: Some(final_diversity.trace),
                rewrites: rewrite_session(plan).trace_records(),
            },
        );
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
    }
}
