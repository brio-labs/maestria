use maestria_domain::{
    EvidenceCandidate, SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace,
    SearchTraceExpansion,
};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};
use crate::types::{
    CandidateRequest, ExpansionPolicy, RankedCandidate, RerankRequest, RetrievalError,
    RetrievalExperiment, RetrievalResult,
};

use crate::sync::SyncPipeline;
pub(super) fn ensure_trace(
    plan: &SearchPlan,
    mut outcome: SearchOutcome,
    lanes: Vec<maestria_domain::SearchTraceLane>,
    fusion_enabled: bool,
    expansion_enabled: bool,
    rerank_trace: Option<maestria_domain::SearchTraceRerank>,
) -> SearchOutcome {
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
            && trace.expansions.len() == usize::from(expansion_enabled)
            && trace.matches_evidence(&outcome.evidence)
    });
    if trace_is_valid {
        return outcome;
    }
    let stop_reason = match &outcome.status {
        SearchStatus::NoEvidenceFound => SearchStopReason::NoEvidence,
        SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
            SearchStopReason::PolicyDenied
        }
        SearchStatus::Abstained => SearchStopReason::Abstained,
        SearchStatus::EvidenceIncomplete
        | SearchStatus::StaleEvidenceOnly
        | SearchStatus::SourcesConflict => SearchStopReason::RequirementsUnmet,
        _ if outcome.evidence.len() >= plan.stop_conditions.max_results as usize => {
            SearchStopReason::ResultsLimit
        }
        _ => SearchStopReason::EvidenceComplete,
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
    trace.rerank = rerank_trace;
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
}

async fn collect_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &SearchQuery,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let mut tasks = JoinSet::new();
    for (index, retriever) in retrievers.iter().enumerate() {
        let retriever = Arc::clone(retriever);
        let request = CandidateRequest {
            plan: plan.clone(),
            query: query.clone(),
        };
        let descriptor = retriever.descriptor();
        tasks.spawn(async move { (index, descriptor, retriever.retrieve(request).await) });
    }

    let mut completed = std::iter::repeat_with(|| None)
        .take(retrievers.len())
        .collect::<Vec<_>>();
    while let Some(result) = tasks.join_next().await {
        let (index, descriptor, result) = result
            .map_err(|error| RetrievalError::Internal(format!("retriever task failed: {error}")))?;
        let batch = match result {
            Ok(mut batch) => {
                batch
                    .candidates
                    .truncate(plan.stop_conditions.max_results as usize);
                batch.descriptor = descriptor;
                if !matches!(
                    batch.status,
                    maestria_domain::SearchLaneStatus::Failed { .. }
                ) {
                    batch.status = if batch.candidates.is_empty() {
                        maestria_domain::SearchLaneStatus::Empty
                    } else {
                        maestria_domain::SearchLaneStatus::Succeeded
                    };
                }
                batch
            }
            Err(RetrievalError::Cancelled) => return Err(RetrievalError::Cancelled),
            Err(error) => crate::types::CandidateBatch {
                descriptor,
                candidates: Vec::new(),
                status: maestria_domain::SearchLaneStatus::Failed {
                    error: error.to_string(),
                },
            },
        };
        completed[index] = Some(batch);
    }

    completed
        .into_iter()
        .map(|batch| {
            batch.ok_or_else(|| {
                RetrievalError::Internal("retriever task produced no result".to_string())
            })
        })
        .collect()
}

fn trace_lanes(batches: &[crate::types::CandidateBatch]) -> Vec<maestria_domain::SearchTraceLane> {
    batches
        .iter()
        .map(|batch| maestria_domain::SearchTraceLane {
            retriever_id: batch.descriptor.id.clone(),
            status: batch.status.clone(),
            candidates: batch
                .candidates
                .iter()
                .enumerate()
                .map(
                    |(rank, candidate)| maestria_domain::SearchTraceLaneCandidate {
                        evidence_id: candidate.evidence_id,
                        artifact_version: candidate.artifact_version,
                        source_span: candidate.source_span.clone(),
                        lane_rank: (rank + 1) as u32,
                        duplicate_cluster: candidate.duplicate_cluster,
                        scores: candidate.scores.clone(),
                        reasons: candidate.reasons.clone(),
                    },
                )
                .collect(),
        })
        .collect()
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
        let batches = collect_batches(&self.retrievers, plan, &query).await?;
        let lanes = trace_lanes(&batches);
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
        if let Some(reranker) = &self.reranker {
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
        let outcome = ensure_trace(
            plan,
            report.outcome,
            lanes,
            self.fusion.is_some(),
            self.expander.is_some(),
            rerank_trace,
        );
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
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
        let lane = maestria_domain::SearchTraceLane {
            retriever_id: "sync_pipeline".to_string(),
            status: if outcome.evidence.is_empty() {
                maestria_domain::SearchLaneStatus::Empty
            } else {
                maestria_domain::SearchLaneStatus::Succeeded
            },
            candidates: outcome
                .evidence
                .iter()
                .enumerate()
                .map(|(i, c)| maestria_domain::SearchTraceLaneCandidate {
                    evidence_id: c.evidence_id,
                    artifact_version: c.artifact_version,
                    source_span: c.source_span.clone(),
                    lane_rank: (i + 1) as u32,
                    duplicate_cluster: c.duplicate_cluster,
                    scores: c.scores.clone(),
                    reasons: c.reasons.clone(),
                })
                .collect(),
        };
        let outcome = ensure_trace(
            plan,
            outcome,
            vec![lane],
            self.pipeline.fusion_enabled(),
            self.pipeline.expander_enabled(),
            None,
        );
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
    }
}
