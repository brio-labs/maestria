use async_trait::async_trait;
use maestria_domain::RerankCandidateStatus;
use maestria_retrieval::bounded_reranker::BoundedReranker;
use maestria_retrieval::traits::{CandidateReranker, RerankScorer};
use maestria_retrieval::types::{
    RankedCandidate, RerankLimits, RerankRequest, RerankScoreComponents, RerankScorerInput,
};
use maestria_retrieval::{RetrievalError, RetrievalResult};
use std::sync::Arc;

use crate::common::{candidate_fixture, dummy_plan};

struct MockScorer {
    model: String,
    fingerprint: maestria_domain::RetrievalModelFingerprint,
}

#[async_trait]
impl RerankScorer for MockScorer {
    fn model(&self) -> String {
        self.model.clone()
    }
    fn fingerprint(&self) -> maestria_domain::RetrievalModelFingerprint {
        self.fingerprint.clone()
    }

    fn compatible_with(&self, _plan: &maestria_domain::RetrievalModelFingerprint) -> bool {
        true
    }
    async fn score(
        &self,
        input: RerankScorerInput,
    ) -> Result<RerankScoreComponents, RetrievalError> {
        let id_val = input.candidate.evidence_id.value();
        if id_val == 999 {
            return Err(RetrievalError::Timeout);
        }
        if id_val == 998 {
            return Err(RetrievalError::Cancelled);
        }
        Ok(RerankScoreComponents {
            relevance: (id_val * 10) as u32,
            constraints: vec![maestria_retrieval::types::RerankConstraintScore {
                name: "query".into(),
                score: (id_val * 5) as u32,
            }],
        })
    }
}

fn create_test_candidate(id: u64, rank: usize) -> RetrievalResult<RankedCandidate> {
    let mut candidate = candidate_fixture()?;
    candidate.evidence_id = maestria_domain::EvidenceId::new(id);
    Ok(RankedCandidate { candidate, rank })
}

#[tokio::test]
async fn test_bounded_reranker_limits_and_trace() -> RetrievalResult<()> {
    let scorer = Arc::new(MockScorer {
        model: "mock-scorer".into(),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("v1".to_string())?,
    });
    let limits = RerankLimits {
        input_cap: 5,
        score_cap: 3,
        output_cap: 2,
    };
    let reranker = BoundedReranker::new(scorer, limits);

    let plan = dummy_plan()?;
    let candidates = vec![
        create_test_candidate(1, 0)?,
        create_test_candidate(2, 1)?,
        create_test_candidate(3, 2)?,
        create_test_candidate(4, 3)?,
        create_test_candidate(5, 4)?,
        create_test_candidate(6, 5)?, // Exceeds input cap
    ];

    let request = RerankRequest {
        plan,
        candidates,
        max_latency_ms: 100,
    };

    let result = reranker.rerank(request).await?;

    assert_eq!(result.candidates.len(), 2);
    assert_eq!(result.candidates[0].candidate.evidence_id.value(), 3);
    assert_eq!(result.candidates[1].candidate.evidence_id.value(), 2);

    let trace = result.trace;
    assert_eq!(trace.candidates.len(), 6);
    assert_eq!(trace.input_cap, 5);
    assert_eq!(trace.score_cap, 3);
    assert_eq!(trace.output_cap, 2);

    let c3 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 3)
        .ok_or(RetrievalError::Internal(
            "missing reranked candidate".into(),
        ))?;
    assert_eq!(c3.status, RerankCandidateStatus::Reranked);
    assert_eq!(c3.relevance_score, Some(30));
    assert_eq!(c3.constraint_score, Some(15));
    assert_eq!(
        c3.constraint_scores,
        vec![maestria_domain::SearchTraceConstraintScore {
            name: "query".into(),
            score: 15,
        }]
    );

    let c1 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 1)
        .ok_or(RetrievalError::Internal(
            "missing first candidate trace".into(),
        ))?;
    assert_eq!(c1.status, RerankCandidateStatus::SkippedCap);

    let c4 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 4)
        .ok_or(RetrievalError::Internal(
            "missing skipped candidate trace".into(),
        ))?;
    assert_eq!(c4.status, RerankCandidateStatus::SkippedCap);
    let c6 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 6)
        .ok_or(RetrievalError::Internal(
            "missing capped candidate trace".into(),
        ))?;
    assert_eq!(c6.status, RerankCandidateStatus::SkippedCap);
    Ok(())
}

#[tokio::test]
async fn test_bounded_reranker_fallback() -> RetrievalResult<()> {
    let scorer = Arc::new(MockScorer {
        model: "mock-scorer".into(),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("v1".to_string())?,
    });
    let limits = RerankLimits {
        input_cap: 5,
        score_cap: 5,
        output_cap: 5,
    };
    let reranker = BoundedReranker::new(scorer, limits);

    let plan = dummy_plan()?;
    let candidates = vec![
        create_test_candidate(1, 0)?,
        create_test_candidate(999, 1)?, // Timeout
        create_test_candidate(2, 2)?,
    ];

    let request = RerankRequest {
        plan,
        candidates,
        max_latency_ms: 100,
    };

    let result = reranker.rerank(request).await?;

    assert_eq!(result.candidates[0].candidate.evidence_id.value(), 2);
    assert_eq!(result.candidates[1].candidate.evidence_id.value(), 1);
    assert_eq!(result.candidates[2].candidate.evidence_id.value(), 999);

    let trace = result.trace;
    let c999 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 999)
        .ok_or(RetrievalError::Internal(
            "missing fallback candidate trace".into(),
        ))?;
    assert!(matches!(
        c999.status,
        RerankCandidateStatus::ErrorFallback(_)
    ));
    assert_eq!(c999.relevance_score, None);
    Ok(())
}

#[tokio::test]
async fn test_bounded_reranker_cancellation() -> RetrievalResult<()> {
    let scorer = Arc::new(MockScorer {
        model: "mock-scorer".into(),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("v1".to_string())?,
    });
    let limits = RerankLimits {
        input_cap: 5,
        score_cap: 5,
        output_cap: 5,
    };
    let reranker = BoundedReranker::new(scorer, limits);

    let plan = dummy_plan()?;
    let candidates = vec![
        create_test_candidate(1, 0)?,
        create_test_candidate(998, 1)?, // Cancelled
    ];

    let request = RerankRequest {
        plan,
        candidates,
        max_latency_ms: 100,
    };

    let result = reranker.rerank(request).await;
    assert!(matches!(result, Err(RetrievalError::Cancelled)));
    Ok(())
}
