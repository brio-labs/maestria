use maestria_domain::{
    SearchExecution, SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionResource,
    SearchExecutionUsage, SearchLaneStatus, SearchPlan,
};

use crate::engine::rewrite_session;
use crate::types::{RetrievalError, RetrievalResult};

fn default_capabilities() -> maestria_governance::SearchCapabilities {
    maestria_governance::SearchCapabilities::new()
        .with_intent(maestria_domain::SearchIntent::ExactLookup)
        .with_intent(maestria_domain::SearchIntent::FactualLocal)
        .with_stage(maestria_domain::SearchStage::InitialRetrieval)
        .with_modality(maestria_domain::Modality::Text)
        .with_snapshot(maestria_domain::CorpusSnapshotId::new(1))
        .with_generation(maestria_domain::IndexGenerationId::new(1))
        .allow_global_scope()
        .max_scope_ids(u32::MAX)
        .max_budgets(1_000, 30_000, 8, 3, 0)
        .with_security_filters()
}

fn execution_candidate_limit(budget: SearchExecutionBudget) -> usize {
    maestria_domain::saturating_usize(budget.max_results())
        .min(maestria_domain::saturating_usize(budget.max_candidates()))
        .min(maestria_domain::saturating_usize(budget.max_work_units()))
}

fn partition_budget(
    global: SearchExecutionBudget,
    lanes: usize,
    lane: usize,
) -> Option<SearchExecutionBudget> {
    let lanes = lanes.max(1) as u64;
    let partition = |total: u64| total / lanes + u64::from((lane as u64) < total % lanes);
    let max_results = partition(global.max_results());
    let max_candidates = partition(global.max_candidates());
    let max_work_units = partition(global.max_work_units());
    if max_results == 0 || max_candidates == 0 || max_work_units == 0 {
        return None;
    }
    let max_bytes_read = global
        .max_bytes_read()
        .map(|limit| partition(limit.get()))
        .and_then(std::num::NonZeroU64::new);
    SearchExecutionBudget::with_byte_limit(
        max_results,
        max_candidates,
        max_work_units,
        max_bytes_read,
    )
    .ok()
}

fn lane_execution(
    budget: SearchExecutionBudget,
    candidate_count: usize,
    truncated: bool,
) -> SearchExecution {
    let count = maestria_domain::saturating_u64(candidate_count);
    let usage = SearchExecutionUsage::new(count, count, count, 0);
    let completion = if truncated {
        let result_limit = budget.max_results();
        let candidate_limit = budget.max_candidates();
        let work_limit = budget.max_work_units();
        if result_limit <= candidate_limit && result_limit <= work_limit {
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::Results)
        } else if candidate_limit <= work_limit {
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::Candidates)
        } else {
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::WorkUnits)
        }
    } else {
        SearchExecutionCompletion::Complete
    };
    SearchExecution::new(budget, usage, completion)
}

fn exhausted_lane_execution(budget: SearchExecutionBudget) -> SearchExecution {
    SearchExecution::new(
        budget,
        SearchExecutionUsage::default(),
        SearchExecutionCompletion::Exhausted(SearchExecutionResource::Candidates),
    )
}

type PipelineRetriever<'a, C> =
    Box<dyn Fn(&SearchPlan, SearchExecutionBudget) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineQueryRetriever<'a, C> =
    Box<dyn Fn(&SearchPlan, &str, SearchExecutionBudget) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineFusion<'a, C> = Box<dyn Fn(Vec<Vec<C>>) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineStage<'a, C> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type SyncLaneSets<C> = Vec<(String, Vec<C>, SearchLaneStatus, SearchExecution)>;
type PipelineCandidateFilter<'a, C> =
    Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<(Vec<C>, SearchLaneStatus)> + 'a>;
type PipelineEvaluator<'a, C, O> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<O> + 'a>;

pub struct SyncPipeline<'a, C, O> {
    retrievers: Vec<PipelineRetriever<'a, C>>,
    query_retrievers: Vec<PipelineQueryRetriever<'a, C>>,
    fusion: Option<PipelineFusion<'a, C>>,
    reranker: Option<PipelineStage<'a, C>>,
    pre_expander: Option<PipelineStage<'a, C>>,
    expander: Option<PipelineStage<'a, C>>,
    candidate_filter: Option<PipelineCandidateFilter<'a, C>>,
    evaluator: PipelineEvaluator<'a, C, O>,
    capabilities: maestria_governance::SearchCapabilities,
    security_policy: maestria_governance::RetrievalSecurityPolicy,
}
impl<'a, C, O> SyncPipeline<'a, C, O> {
    pub fn new<R, V>(retrievers: Vec<R>, evaluator: V) -> Self
    where
        R: Fn(&SearchPlan, SearchExecutionBudget) -> RetrievalResult<Vec<C>> + 'a,
        V: Fn(Vec<C>, &SearchPlan) -> RetrievalResult<O> + 'a,
    {
        Self {
            retrievers: retrievers
                .into_iter()
                .map(|retriever| Box::new(retriever) as _)
                .collect(),
            query_retrievers: Vec::new(),
            fusion: None,
            reranker: None,
            pre_expander: None,
            expander: None,
            candidate_filter: None,
            evaluator: Box::new(evaluator),
            capabilities: default_capabilities(),
            security_policy: maestria_governance::RetrievalSecurityPolicy::default(),
        }
    }
    pub fn with_capabilities(
        mut self,
        capabilities: maestria_governance::SearchCapabilities,
    ) -> Self {
        self.capabilities = capabilities;
        self
    }
    pub fn with_security_policy(
        mut self,
        security_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self {
        self.security_policy = security_policy;
        self
    }
    pub fn with_candidate_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(Vec<C>, &SearchPlan) -> RetrievalResult<(Vec<C>, SearchLaneStatus)> + 'a,
    {
        self.candidate_filter = Some(Box::new(filter));
        self
    }
    pub fn with_query_retriever<F>(mut self, retriever: F) -> Self
    where
        F: Fn(&SearchPlan, &str, SearchExecutionBudget) -> RetrievalResult<Vec<C>> + 'a,
    {
        self.query_retrievers.push(Box::new(retriever));
        self
    }

    pub fn with_fusion<F>(mut self, fusion: F) -> Self
    where
        F: Fn(Vec<Vec<C>>) -> RetrievalResult<Vec<C>> + 'a,
    {
        self.fusion = Some(Box::new(fusion));
        self
    }

    pub fn with_reranker<F>(mut self, reranker: F) -> Self
    where
        F: Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a,
    {
        self.reranker = Some(Box::new(reranker));
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Reranking);
        self
    }

    pub fn with_pre_expander<F>(mut self, pre_expander: F) -> Self
    where
        F: Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a,
    {
        self.pre_expander = Some(Box::new(pre_expander));
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Filtering);
        self
    }

    pub fn with_expander<F>(mut self, expander: F) -> Self
    where
        F: Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a,
    {
        self.expander = Some(Box::new(expander));
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Filtering);
        self
    }

    pub(crate) fn fusion_enabled(&self) -> bool {
        self.fusion.is_some()
    }

    pub(crate) fn expander_enabled(&self) -> bool {
        self.expander.is_some()
    }
    pub(crate) fn query_rewrites_enabled(&self) -> bool {
        !self.query_retrievers.is_empty()
    }
    fn apply_candidate_filter(
        &self,
        candidates: Vec<C>,
        plan: &SearchPlan,
    ) -> RetrievalResult<(Vec<C>, SearchLaneStatus)> {
        if let Some(filter) = &self.candidate_filter {
            return filter(candidates, plan);
        }
        let status = if candidates.is_empty() {
            SearchLaneStatus::Empty
        } else {
            SearchLaneStatus::Succeeded
        };
        Ok((candidates, status))
    }

    fn collect_lane_sets(
        &self,
        plan: &SearchPlan,
        execution_budget: SearchExecutionBudget,
        rewrite_queries: &[String],
        lane_count: usize,
        check_timeout: &dyn Fn() -> RetrievalResult<()>,
    ) -> RetrievalResult<(Vec<Vec<C>>, SyncLaneSets<C>)>
    where
        C: Clone,
    {
        let mut sets = Vec::with_capacity(lane_count);
        let mut lane_sets = Vec::with_capacity(lane_count);
        let mut lane_index = 0_usize;
        for retriever in &self.retrievers {
            let Some(lane_budget) = partition_budget(execution_budget, lane_count, lane_index)
            else {
                lane_sets.push((
                    plan.original_query.clone(),
                    Vec::new(),
                    SearchLaneStatus::Failed {
                        error: "lane budget exhausted before dispatch".to_string(),
                    },
                    exhausted_lane_execution(execution_budget),
                ));
                sets.push(Vec::new());
                lane_index = lane_index.saturating_add(1);
                continue;
            };
            let candidate_limit = execution_candidate_limit(lane_budget);
            let mut set = retriever(plan, lane_budget)?;
            let truncated = set.len() > candidate_limit;
            set.truncate(candidate_limit);
            let execution = lane_execution(lane_budget, set.len(), truncated);
            let (set, status) = self.apply_candidate_filter(set, plan)?;
            lane_sets.push((plan.original_query.clone(), set.clone(), status, execution));
            sets.push(set);
            lane_index = lane_index.saturating_add(1);
            check_timeout()?;
        }
        for query in rewrite_queries {
            for retriever in &self.query_retrievers {
                let Some(lane_budget) = partition_budget(execution_budget, lane_count, lane_index)
                else {
                    lane_sets.push((
                        query.clone(),
                        Vec::new(),
                        SearchLaneStatus::Failed {
                            error: "lane budget exhausted before dispatch".to_string(),
                        },
                        exhausted_lane_execution(execution_budget),
                    ));
                    sets.push(Vec::new());
                    lane_index = lane_index.saturating_add(1);
                    continue;
                };
                let candidate_limit = execution_candidate_limit(lane_budget);
                let mut set = retriever(plan, query, lane_budget)?;
                let truncated = set.len() > candidate_limit;
                set.truncate(candidate_limit);
                let execution = lane_execution(lane_budget, set.len(), truncated);
                let (set, status) = self.apply_candidate_filter(set, plan)?;
                lane_sets.push((query.clone(), set.clone(), status, execution));
                sets.push(set);
                lane_index = lane_index.saturating_add(1);
                check_timeout()?;
            }
        }
        Ok((sets, lane_sets))
    }

    pub fn run(&self, plan: &SearchPlan) -> RetrievalResult<O>
    where
        C: Clone,
    {
        self.run_with_trace(plan).map(|(output, _)| output)
    }
    pub(crate) fn run_with_trace(&self, plan: &SearchPlan) -> RetrievalResult<(O, SyncLaneSets<C>)>
    where
        C: Clone,
    {
        maestria_governance::SearchPlanValidator::validate(
            plan,
            &self.capabilities,
            &self.security_policy,
        )
        .map_err(RetrievalError::SearchPlan)?;
        let execution_budget = plan
            .execution_budget()
            .map_err(RetrievalError::Compatibility)?;
        let candidate_limit = execution_candidate_limit(execution_budget);
        let start = crate::MonotonicInstant::now();
        let timeout_ms = plan.budgets.max_latency_ms() as u64;
        let check_timeout = || -> RetrievalResult<()> {
            let elapsed = start.elapsed();
            if timeout_ms > 0 && elapsed.as_millis() as u64 > timeout_ms {
                Err(RetrievalError::Timeout)
            } else {
                Ok(())
            }
        };
        if self.retrievers.is_empty() {
            return Err(RetrievalError::Internal("No retrievers configured".into()));
        }
        let rewrite_queries = if self.query_retrievers.is_empty() {
            Vec::new()
        } else {
            rewrite_session(plan)
                .records()
                .iter()
                .filter(|record| {
                    record.origin == crate::rewrite::RewriteOrigin::Deterministic
                        && record.stage == crate::rewrite::StageRole::InitialRetrieval
                })
                .map(|record| record.query.clone())
                .collect::<Vec<_>>()
        };
        let lane_count = self.retrievers.len().saturating_add(
            self.query_retrievers
                .len()
                .saturating_mul(rewrite_queries.len()),
        );
        let (sets, lane_sets) = self.collect_lane_sets(
            plan,
            execution_budget,
            &rewrite_queries,
            lane_count,
            &check_timeout,
        )?;
        let mut candidates = if let Some(fusion) = &self.fusion {
            let mut fused = fusion(sets)?;
            fused.truncate(candidate_limit);
            check_timeout()?;
            fused
        } else {
            sets.into_iter().flatten().take(candidate_limit).collect()
        };
        if plan
            .stages
            .contains(&maestria_domain::SearchStage::Reranking)
            && let Some(reranker) = &self.reranker
        {
            candidates = reranker(candidates, plan)?;
            candidates.truncate(candidate_limit);
            check_timeout()?;
        }
        if plan
            .stages
            .contains(&maestria_domain::SearchStage::Filtering)
        {
            if let Some(pre_expander) = &self.pre_expander {
                candidates = pre_expander(candidates, plan)?;
                candidates.truncate(candidate_limit);
                check_timeout()?;
            }
            if let Some(expander) = &self.expander {
                candidates = expander(candidates, plan)?;
                candidates.truncate(candidate_limit);
                check_timeout()?;
            }
        }
        candidates.truncate(candidate_limit);
        let output = (self.evaluator)(candidates, plan)?;
        check_timeout()?;
        Ok((output, lane_sets))
    }
}
