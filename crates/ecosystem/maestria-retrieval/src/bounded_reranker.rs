use crate::traits::{CandidateReranker, RerankScorer};
use crate::types::{
    RankedCandidate, RerankLimits, RerankRequest, RerankResult, RerankScoreComponents,
    RerankScorerInput, RetrievalError,
};
use async_trait::async_trait;
use maestria_domain::{
    RerankCandidateStatus, SearchTraceConstraintScore, SearchTraceRerank,
    SearchTraceRerankCandidate,
};
use std::sync::Arc;
use std::time::Duration;

pub struct BoundedReranker {
    scorer: Arc<dyn RerankScorer>,
    limits: RerankLimits,
}

impl BoundedReranker {
    pub fn new(scorer: Arc<dyn RerankScorer>, limits: RerankLimits) -> Self {
        Self { scorer, limits }
    }
}

struct ScoredCandidate {
    ranked: RankedCandidate,
    components: RerankScoreComponents,
    total_score: u32,
    fallback_error: Option<String>,
}

#[allow(clippy::disallowed_methods)]
async fn score_candidates(
    scorer: &dyn RerankScorer,
    plan: &maestria_domain::SearchPlan,
    candidates: Vec<RankedCandidate>,
    score_cap: usize,
    max_latency_ms: u32,
) -> Result<(Vec<ScoredCandidate>, Vec<SearchTraceRerankCandidate>), RetrievalError> {
    let started = std::time::Instant::now();
    let max_duration = Duration::from_millis(u64::from(max_latency_ms));
    let mut budget_exhausted = false;
    let mut scored = Vec::new();
    let mut skipped = Vec::new();

    for (index, ranked) in candidates.into_iter().enumerate() {
        if index >= score_cap {
            skipped.push(skipped_trace(&ranked));
            continue;
        }
        let input = RerankScorerInput {
            plan: plan.clone(),
            candidate: ranked.candidate.clone(),
        };
        let result = if budget_exhausted {
            Err(RetrievalError::Timeout)
        } else {
            let elapsed = started.elapsed();
            if elapsed >= max_duration {
                budget_exhausted = true;
                Err(RetrievalError::Timeout)
            } else {
                tokio::time::timeout(max_duration.saturating_sub(elapsed), scorer.score(input))
                    .await
                    .map_err(|_| RetrievalError::Timeout)
            }
        };
        if matches!(&result, Err(RetrievalError::Timeout)) {
            budget_exhausted = true;
        }
        match result {
            Ok(Ok(mut components)) => {
                components.constraints.sort_by(|left, right| {
                    left.name
                        .cmp(&right.name)
                        .then(left.score.cmp(&right.score))
                });
                let constraint_total = components
                    .constraints
                    .iter()
                    .fold(0u32, |total, constraint| {
                        total.saturating_add(constraint.score)
                    });
                scored.push(ScoredCandidate {
                    ranked,
                    total_score: components.relevance.saturating_add(constraint_total),
                    components,
                    fallback_error: None,
                });
            }
            Ok(Err(RetrievalError::Cancelled)) => return Err(RetrievalError::Cancelled),
            Ok(Err(error)) | Err(error) => scored.push(ScoredCandidate {
                ranked,
                components: RerankScoreComponents {
                    relevance: 0,
                    constraints: Vec::new(),
                },
                total_score: 0,
                fallback_error: Some(error.to_string()),
            }),
        }
    }
    Ok((scored, skipped))
}

fn skipped_trace(ranked: &RankedCandidate) -> SearchTraceRerankCandidate {
    SearchTraceRerankCandidate {
        candidate_id: ranked.candidate.evidence_id,
        original_rank: ranked.rank,
        new_rank: None,
        status: RerankCandidateStatus::SkippedCap,
        relevance_score: None,
        constraint_score: None,
        constraint_scores: Vec::new(),
    }
}

fn finish_candidates(
    mut scored: Vec<ScoredCandidate>,
    output_cap: usize,
    mut trace: Vec<SearchTraceRerankCandidate>,
) -> (Vec<RankedCandidate>, Vec<SearchTraceRerankCandidate>) {
    scored.sort_by(|left, right| {
        right
            .total_score
            .cmp(&left.total_score)
            .then(left.ranked.rank.cmp(&right.ranked.rank))
            .then(
                left.ranked
                    .candidate
                    .evidence_id
                    .cmp(&right.ranked.candidate.evidence_id),
            )
    });
    let mut final_candidates = Vec::new();
    for mut item in scored {
        let original_rank = item.ranked.rank;
        let candidate_id = item.ranked.candidate.evidence_id;
        let fallback = item.fallback_error.is_some();
        let mut status = item.fallback_error.map_or(
            RerankCandidateStatus::Reranked,
            RerankCandidateStatus::ErrorFallback,
        );
        let new_rank = if final_candidates.len() < output_cap {
            let rank = final_candidates.len();
            item.ranked.rank = rank;
            final_candidates.push(item.ranked);
            Some(rank)
        } else {
            None
        };
        if new_rank.is_none() && !fallback {
            status = RerankCandidateStatus::SkippedCap;
        }
        let reranked = matches!(&status, RerankCandidateStatus::Reranked);
        let constraint_score = reranked.then(|| {
            item.components
                .constraints
                .iter()
                .fold(0u32, |total, constraint| {
                    total.saturating_add(constraint.score)
                })
        });
        let constraint_scores = if reranked {
            item.components
                .constraints
                .iter()
                .map(|constraint| SearchTraceConstraintScore {
                    name: constraint.name.clone(),
                    score: constraint.score,
                })
                .collect()
        } else {
            Vec::new()
        };
        trace.push(SearchTraceRerankCandidate {
            candidate_id,
            original_rank,
            new_rank,
            status,
            relevance_score: reranked.then_some(item.components.relevance),
            constraint_score,
            constraint_scores,
        });
    }
    trace.sort_by(|left, right| {
        left.candidate_id
            .cmp(&right.candidate_id)
            .then(left.original_rank.cmp(&right.original_rank))
            .then(left.new_rank.cmp(&right.new_rank))
    });
    (final_candidates, trace)
}

#[async_trait]
impl CandidateReranker for BoundedReranker {
    async fn rerank(&self, request: RerankRequest) -> Result<RerankResult, RetrievalError> {
        if !self.scorer.compatible_with(&request.plan.fingerprint) {
            return Err(RetrievalError::Compatibility(
                maestria_domain::SearchCompatibilityError::ModelFingerprintMismatch {
                    expected: request.plan.fingerprint,
                    found: self.scorer.fingerprint(),
                },
            ));
        }
        let RerankRequest {
            plan,
            candidates,
            max_latency_ms,
        } = request;
        let mut remaining = candidates.into_iter();
        let input_candidates = remaining.by_ref().take(self.limits.input_cap).collect();
        let mut trace: Vec<SearchTraceRerankCandidate> =
            remaining.map(|ranked| skipped_trace(&ranked)).collect();
        let (scored, skipped) = score_candidates(
            self.scorer.as_ref(),
            &plan,
            input_candidates,
            self.limits.score_cap,
            max_latency_ms,
        )
        .await?;
        trace.extend(skipped);
        let (candidates, trace) = finish_candidates(scored, self.limits.output_cap, trace);
        Ok(RerankResult {
            candidates,
            trace: SearchTraceRerank {
                model: self.scorer.model(),
                fingerprint: self.scorer.fingerprint(),
                input_cap: self.limits.input_cap,
                score_cap: self.limits.score_cap,
                output_cap: self.limits.output_cap,
                candidates: trace,
            },
        })
    }
}
