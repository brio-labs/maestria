use maestria_domain::SearchPlan;

use crate::types::{RetrievalError, RetrievalResult};

type PipelineRetriever<'a, C> = Box<dyn Fn(&SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineFusion<'a, C> = Box<dyn Fn(Vec<Vec<C>>) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineStage<'a, C> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineEvaluator<'a, C, O> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<O> + 'a>;

pub struct SyncPipeline<'a, C, O> {
    retrievers: Vec<PipelineRetriever<'a, C>>,
    fusion: Option<PipelineFusion<'a, C>>,
    reranker: Option<PipelineStage<'a, C>>,
    expander: Option<PipelineStage<'a, C>>,
    evaluator: PipelineEvaluator<'a, C, O>,
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
            fusion: None,
            reranker: None,
            expander: None,
            evaluator: Box::new(evaluator),
        }
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
        self
    }

    pub fn with_expander<F>(mut self, expander: F) -> Self
    where
        F: Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a,
    {
        self.expander = Some(Box::new(expander));
        self
    }
    pub(crate) fn fusion_enabled(&self) -> bool {
        self.fusion.is_some()
    }

    pub(crate) fn expander_enabled(&self) -> bool {
        self.expander.is_some()
    }

    #[allow(clippy::disallowed_methods)]
    pub fn run(&self, plan: &SearchPlan) -> RetrievalResult<O> {
        let start = std::time::Instant::now();
        let timeout_ms = plan.budgets.max_latency_ms() as u64;
        let check_timeout = || -> RetrievalResult<()> {
            if timeout_ms > 0 && start.elapsed().as_millis() as u64 > timeout_ms {
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
        let mut candidates = if let Some(fusion) = &self.fusion {
            let fused = fusion(sets)?;
            check_timeout()?;
            fused
        } else {
            sets.into_iter().flatten().collect()
        };
        if let Some(reranker) = &self.reranker {
            candidates = reranker(candidates, plan)?;
            check_timeout()?;
        }
        if let Some(expander) = &self.expander {
            candidates = expander(candidates, plan)?;
            check_timeout()?;
        }
        candidates.truncate(plan.stop_conditions.max_results as usize);
        let output = (self.evaluator)(candidates, plan)?;
        check_timeout()?;
        Ok(output)
    }
}
