use maestria_domain::{EvidenceCandidate, SearchOutcome, SearchPlan};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use std::time::Duration;

use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};
use crate::types::{
    CandidateRequest, ExpansionPolicy, RankedCandidate, RerankRequest, RetrievalError,
    RetrievalExperiment, RetrievalResult,
};

type PipelineRetriever<'a, C> = Box<dyn Fn(&SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineFusion<'a, C> = Box<dyn Fn(Vec<Vec<C>>) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineStage<'a, C> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<Vec<C>> + 'a>;
type PipelineEvaluator<'a, C, O> = Box<dyn Fn(Vec<C>, &SearchPlan) -> RetrievalResult<O> + 'a>;

pub struct RetrievalEngine {
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    fusion: Option<Arc<dyn RankFusion>>,
    reranker: Option<Arc<dyn CandidateReranker>>,
    expander: Option<Arc<dyn ContextExpander>>,
    evaluator: Arc<dyn RetrievalEvaluator>,
}

impl RetrievalEngine {
    pub fn new(
        retrievers: Vec<Arc<dyn CandidateRetriever>>,
        evaluator: Arc<dyn RetrievalEvaluator>,
    ) -> Self {
        Self {
            retrievers,
            fusion: None,
            reranker: None,
            expander: None,
            evaluator,
        }
    }

    pub fn with_fusion(mut self, fusion: Arc<dyn RankFusion>) -> Self {
        self.fusion = Some(fusion);
        self
    }

    pub fn with_reranker(mut self, reranker: Arc<dyn CandidateReranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    pub fn with_expander(mut self, expander: Arc<dyn ContextExpander>) -> Self {
        self.expander = Some(expander);
        self
    }

    pub async fn search(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        let timeout_ms = plan.budgets.max_latency_ms() as u64;
        let search = self.search_internal(plan);
        if timeout_ms > 0 {
            tokio::time::timeout(Duration::from_millis(timeout_ms), search)
                .await
                .map_err(|_| RetrievalError::Timeout)?
        } else {
            search.await
        }
    }

    async fn search_internal(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        if self.retrievers.is_empty() {
            return Err(RetrievalError::Internal("No retrievers configured".into()));
        }
        let query = SearchQuery {
            q: plan.original_query.clone(),
            limit: plan.stop_conditions.max_results as usize,
            offset: 0,
        };
        let mut batches = Vec::with_capacity(self.retrievers.len());
        for retriever in &self.retrievers {
            let request = CandidateRequest {
                plan: plan.clone(),
                query: query.clone(),
            };
            let descriptor = retriever.descriptor();
            let mut batch = retriever.retrieve(request).await?;
            batch
                .candidates
                .truncate(plan.stop_conditions.max_results as usize);
            batch.descriptor = descriptor;
            batches.push(batch);
        }

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
                .flat_map(|batch| batch.candidates)
                .enumerate()
                .map(|(rank, candidate)| RankedCandidate { candidate, rank })
                .collect()
        };

        if let Some(reranker) = &self.reranker {
            ranked = reranker
                .rerank(RerankRequest {
                    plan: plan.clone(),
                    candidates: ranked,
                })
                .await?
                .candidates;
        }

        let mut candidates = if let Some(expander) = &self.expander {
            expander.expand(
                &ranked,
                &ExpansionPolicy {
                    max_results: plan.stop_conditions.max_results as usize,
                    max_depth: plan.stages.len(),
                },
            )?
        } else {
            ranked
                .into_iter()
                .map(|candidate| candidate.candidate)
                .collect()
        };
        candidates.truncate(plan.stop_conditions.max_results as usize);
        let report = self
            .evaluator
            .evaluate(RetrievalExperiment {
                plan: plan.clone(),
                candidates,
            })
            .await?;
        report.outcome.verify_compatibility(plan)?;
        Ok(report.outcome)
    }
}

pub struct SyncRetrievalEngine<'a> {
    pipeline: SyncPipeline<'a, EvidenceCandidate, SearchOutcome>,
}

impl<'a> SyncRetrievalEngine<'a> {
    pub fn new<R, V>(retrievers: Vec<R>, evaluator: V) -> Self
    where
        R: Fn(&SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
        V: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<SearchOutcome> + 'a,
    {
        Self {
            pipeline: SyncPipeline::new(retrievers, evaluator),
        }
    }

    pub fn with_fusion<F>(mut self, fusion: F) -> Self
    where
        F: Fn(Vec<Vec<EvidenceCandidate>>) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_fusion(fusion);
        self
    }

    pub fn with_reranker<F>(mut self, reranker: F) -> Self
    where
        F: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_reranker(reranker);
        self
    }

    pub fn with_expander<F>(mut self, expander: F) -> Self
    where
        F: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_expander(expander);
        self
    }

    pub fn search_sync(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        let outcome = self.pipeline.run(plan)?;
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
    }
}

/// Synchronous orchestration for adapters whose candidate representation is local
/// to the application boundary.
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
