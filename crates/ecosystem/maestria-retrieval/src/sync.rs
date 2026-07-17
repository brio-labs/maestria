use maestria_domain::SearchPlan;

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

type PipelineRetriever<'a, C> = Box<dyn Fn(&SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineQueryRetriever<'a, C> = Box<dyn Fn(&SearchPlan, &str) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineFusion<'a, C> = Box<dyn Fn(Vec<Vec<C>>) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineStage<'a, C> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineEvaluator<'a, C, O> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<O> + 'a>;

pub struct SyncPipeline<'a, C, O> {
    retrievers: Vec<PipelineRetriever<'a, C>>,
    query_retrievers: Vec<PipelineQueryRetriever<'a, C>>,
    fusion: Option<PipelineFusion<'a, C>>,
    reranker: Option<PipelineStage<'a, C>>,
    pre_expander: Option<PipelineStage<'a, C>>,
    expander: Option<PipelineStage<'a, C>>,
    evaluator: PipelineEvaluator<'a, C, O>,
    capabilities: maestria_governance::SearchCapabilities,
}

impl<'a, C, O> SyncPipeline<'a, C, O> {
    pub fn new<R, V>(retrievers: Vec<R>, evaluator: V) -> Self
    where
        R: Fn(&SearchPlan) -> RetrievalResult<Vec<C>> + 'a,
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
            evaluator: Box::new(evaluator),
            capabilities: default_capabilities(),
        }
    }
    pub fn with_capabilities(
        mut self,
        capabilities: maestria_governance::SearchCapabilities,
    ) -> Self {
        self.capabilities = capabilities;
        self
    }
    pub fn with_query_retriever<F>(mut self, retriever: F) -> Self
    where
        F: Fn(&SearchPlan, &str) -> RetrievalResult<Vec<C>> + 'a,
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

    pub fn run(&self, plan: &SearchPlan) -> RetrievalResult<O> {
        maestria_governance::SearchPlanValidator::validate(
            plan,
            &self.capabilities,
            &maestria_governance::RetrievalSecurityPolicy::default(),
        )
        .map_err(RetrievalError::SearchPlan)?;
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
        let mut sets = Vec::with_capacity(self.retrievers.len());
        for retriever in &self.retrievers {
            let mut set = retriever(plan)?;
            set.truncate(plan.stop_conditions.max_results as usize);
            sets.push(set);
            check_timeout()?;
        }
        if !self.query_retrievers.is_empty() {
            let session = rewrite_session(plan);
            for rewrite in session.records().iter().filter(|record| {
                record.origin == crate::rewrite::RewriteOrigin::Deterministic
                    && record.stage == crate::rewrite::StageRole::InitialRetrieval
            }) {
                for retriever in &self.query_retrievers {
                    let mut set = retriever(plan, &rewrite.query)?;
                    set.truncate(plan.stop_conditions.max_results as usize);
                    sets.push(set);
                    check_timeout()?;
                }
            }
        }
        let mut candidates = if let Some(fusion) = &self.fusion {
            let fused = fusion(sets)?;
            check_timeout()?;
            fused
        } else {
            sets.into_iter().flatten().collect()
        };
        if plan
            .stages
            .contains(&maestria_domain::SearchStage::Reranking)
            && let Some(reranker) = &self.reranker
        {
            candidates = reranker(candidates, plan)?;
            check_timeout()?;
        }
        if plan
            .stages
            .contains(&maestria_domain::SearchStage::Filtering)
        {
            if let Some(pre_expander) = &self.pre_expander {
                candidates = pre_expander(candidates, plan)?;
                check_timeout()?;
            }
            if let Some(expander) = &self.expander {
                candidates = expander(candidates, plan)?;
                check_timeout()?;
            }
        }
        candidates.truncate(plan.stop_conditions.max_results as usize);
        let output = (self.evaluator)(candidates, plan)?;
        check_timeout()?;
        Ok(output)
    }
}
