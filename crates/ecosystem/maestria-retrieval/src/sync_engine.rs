use maestria_domain::{
    EvidenceCandidate, SearchLaneStatus, SearchOutcome, SearchPlan, SearchTraceLane,
    SearchTraceLaneCandidate,
};

use crate::engine::{EnsureTraceOptions, ensure_trace, reconcile_status};
use crate::sync::SyncPipeline;
use crate::types::{RankedCandidate, RetrievalResult};

pub struct SyncRetrievalEngine<'a> {
    pipeline: SyncPipeline<'a, EvidenceCandidate, SearchOutcome>,
}

impl<'a> SyncRetrievalEngine<'a> {
    pub fn new<R, V>(retrievers: Vec<R>, evaluator: V) -> Self
    where
        R: Fn(&SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
        V: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<SearchOutcome> + 'a,
    {
        let pipeline =
            SyncPipeline::new(retrievers, evaluator).with_pre_expander(|candidates, plan| {
                let ranked = candidates
                    .into_iter()
                    .enumerate()
                    .map(|(rank, candidate)| RankedCandidate { candidate, rank })
                    .collect::<Vec<_>>();
                Ok(crate::diversity::select_candidates(&ranked, plan)
                    .candidates
                    .into_iter()
                    .map(|candidate| candidate.candidate)
                    .collect())
            });
        Self { pipeline }
    }
    pub fn with_query_retriever<F>(mut self, retriever: F) -> Self
    where
        F: Fn(&SearchPlan, &str) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_query_retriever(retriever);
        self
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
        let (mut outcome, lane_sets) = self.pipeline.run_with_trace(plan)?;
        let ranked = outcome
            .evidence
            .iter()
            .cloned()
            .enumerate()
            .map(|(rank, candidate)| RankedCandidate { candidate, rank })
            .collect::<Vec<_>>();
        let diversity = crate::diversity::select_candidates(&ranked, plan);
        outcome.evidence = diversity
            .candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect();
        outcome.coverage = diversity.coverage;
        outcome.status = reconcile_status(&outcome.status, &diversity.status);
        let lanes = lane_sets
            .into_iter()
            .map(|(query, candidates)| SearchTraceLane {
                retriever_id: "sync_pipeline".to_string(),
                generation: None,
                query,
                status: if candidates.is_empty() {
                    SearchLaneStatus::Empty
                } else {
                    SearchLaneStatus::Succeeded
                },
                candidates: candidates
                    .into_iter()
                    .enumerate()
                    .map(|(index, candidate)| SearchTraceLaneCandidate {
                        evidence_id: candidate.evidence_id,
                        artifact_version: candidate.artifact_version,
                        source_span: candidate.source_span,
                        lane_rank: (index + 1) as u32,
                        duplicate_cluster: candidate.duplicate_cluster,
                        scores: candidate.scores,
                        reasons: candidate.reasons,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let rewrites = if self.pipeline.query_rewrites_enabled() {
            crate::engine::rewrite_session(plan).trace_records()
        } else {
            crate::rewrite::QueryRewriteSession::with_limits(
                &plan.original_query,
                plan.budgets.max_tokens() as usize,
                plan.budgets.max_latency_ms(),
                plan.budgets.max_queries(),
            )
            .trace_records()
        };
        let outcome = ensure_trace(
            plan,
            outcome,
            lanes,
            EnsureTraceOptions {
                fusion_enabled: self.pipeline.fusion_enabled(),
                expansion_enabled: self.pipeline.expander_enabled(),
                rerank_trace: None,
                diversity_trace: Some(diversity.trace),
                rewrites,
                explicit_stop_reason: None,
            },
        );
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
    }
}
