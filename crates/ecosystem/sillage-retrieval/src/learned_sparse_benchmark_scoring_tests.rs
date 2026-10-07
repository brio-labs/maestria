//! Deterministic scoring tests (shared behavior family for the scorer).

use super::*;
use crate::learned_sparse_benchmark::LearnedSparseAcceptedSpan;

fn span(source_id: &str, start: u32, end: u32) -> LearnedSparseAcceptedSpan {
    LearnedSparseAcceptedSpan {
        source_id: source_id.to_string(),
        start,
        end,
    }
}

fn accepted() -> Vec<LearnedSparseAcceptedSpan> {
    vec![
        span("alpha", 10, 20),
        span("alpha", 100, 120),
        span("beta", 0, 30),
    ]
}

fn evidence_expected() -> LearnedSparseExpectedOutcome {
    LearnedSparseExpectedOutcome::Evidence {
        accepted_spans: accepted(),
        evidence_chain: vec!["alpha".to_string(), "beta".to_string()],
        minimum_source_diversity: 2,
    }
}

fn candidate(
    evidence_id: &str,
    rank: u32,
    source_id: &str,
    start: u32,
    end: u32,
    grade: Option<u8>,
) -> LearnedSparseRetrievedCandidate {
    LearnedSparseRetrievedCandidate {
        evidence_id: evidence_id.to_string(),
        lane_rank: rank,
        span: LearnedSparseRetrievedSpan {
            source_id: source_id.to_string(),
            start,
            end,
        },
        citation: Some(LearnedSparseRetrievedSpan {
            source_id: source_id.to_string(),
            start,
            end,
        }),
        grade,
    }
}

#[test]
fn perfect_evidence_retrieval_scores_one() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = score_case(
        "c1",
        &evidence_expected(),
        &[
            candidate("e1", 1, "alpha", 10, 20, Some(2)),
            candidate("e2", 2, "alpha", 100, 120, Some(2)),
            candidate("e3", 3, "beta", 0, 30, Some(2)),
        ],
    )?;
    for metric in [
        &metrics.recall_at_5,
        &metrics.recall_at_20,
        &metrics.recall_at_50,
        &metrics.recall_at_100,
        &metrics.ndcg_at_10,
        &metrics.ndcg_at_20,
        &metrics.mrr_at_10,
        &metrics.mean_average_precision,
        &metrics.exact_span_recall,
        &metrics.evidence_chain_coverage,
        &metrics.source_diversity,
        &metrics.citation_precision,
        &metrics.citation_recall,
        &metrics.abstention_precision,
        &metrics.abstention_recall,
    ] {
        assert_eq!(metric.measured_value(), Some(&Metric::ONE));
    }
    assert_eq!(
        metrics.unsupported_claim_status.measured_value(),
        Some(&CheckStatus::Passed)
    );
    Ok(())
}

#[test]
fn empty_route_on_evidence_case_scores_zero_and_fails_checks()
-> Result<(), Box<dyn std::error::Error>> {
    let metrics = score_case("c2", &evidence_expected(), &[])?;
    assert_eq!(metrics.recall_at_5.measured_value(), Some(&Metric::ZERO));
    assert_eq!(
        metrics.abstention_precision.measured_value(),
        Some(&Metric::ZERO)
    );
    assert_eq!(
        metrics.unsupported_claim_status.measured_value(),
        Some(&CheckStatus::Failed)
    );
    Ok(())
}

#[test]
fn abstain_case_rewards_abstention_and_penalizes_answers() -> Result<(), Box<dyn std::error::Error>>
{
    let expected = LearnedSparseExpectedOutcome::Abstain;
    let metrics = score_case("c3", &expected, &[])?;
    assert_eq!(
        metrics.abstention_precision.measured_value(),
        Some(&Metric::ONE)
    );
    assert_eq!(
        metrics.abstention_recall.measured_value(),
        Some(&Metric::ONE)
    );
    let answered = score_case(
        "c3",
        &expected,
        &[candidate("e1", 1, "alpha", 10, 20, None)],
    )?;
    assert_eq!(
        answered.abstention_precision.measured_value(),
        Some(&Metric::ZERO)
    );
    assert_eq!(
        answered.abstention_recall.measured_value(),
        Some(&Metric::ZERO)
    );
    Ok(())
}

#[test]
fn unsupported_claim_pass_only_when_abstained() -> Result<(), Box<dyn std::error::Error>> {
    let expected = LearnedSparseExpectedOutcome::UnsupportedClaim;
    let abstained = score_case("c4", &expected, &[])?;
    assert_eq!(
        abstained.unsupported_claim_status.measured_value(),
        Some(&CheckStatus::Passed)
    );
    let answered = score_case(
        "c4",
        &expected,
        &[candidate("e1", 1, "alpha", 10, 20, None)],
    )?;
    assert_eq!(
        answered.unsupported_claim_status.measured_value(),
        Some(&CheckStatus::Failed)
    );
    Ok(())
}

#[test]
fn conflict_detection_pass_only_when_abstained() -> Result<(), Box<dyn std::error::Error>> {
    let expected = LearnedSparseExpectedOutcome::Conflict;
    let abstained = score_case("c5", &expected, &[])?;
    assert_eq!(
        abstained.conflict_detection_status.measured_value(),
        Some(&CheckStatus::Passed)
    );
    let answered = score_case(
        "c5",
        &expected,
        &[candidate("e1", 1, "alpha", 10, 20, None)],
    )?;
    assert_eq!(
        answered.conflict_detection_status.measured_value(),
        Some(&CheckStatus::Failed)
    );
    Ok(())
}

#[test]
fn partial_recall_and_ndcg_are_rank_aware() -> Result<(), Box<dyn std::error::Error>> {
    // Only the third accepted span is retrieved, at rank 3.
    let metrics = score_case(
        "c6",
        &evidence_expected(),
        &[candidate("e3", 3, "beta", 0, 30, Some(2))],
    )?;
    assert_eq!(
        metrics.recall_at_5.measured_value(),
        Some(&Metric::from_ratio(1, 3))
    );
    let recall = metrics
        .exact_span_recall
        .measured_value()
        .copied()
        .ok_or("exact span recall is unmeasured")?;
    assert_eq!(recall.value(), Metric::ONE.value() / 3);
    let mrr = metrics
        .mrr_at_10
        .measured_value()
        .copied()
        .ok_or("mrr is unmeasured")?;
    assert_eq!(mrr.value(), Metric::ONE.value() / 3);
    Ok(())
}

#[test]
fn rejects_zero_or_unattributed_ranks() -> Result<(), Box<dyn std::error::Error>> {
    let result = score_case(
        "c7",
        &evidence_expected(),
        &[candidate("e1", 0, "alpha", 10, 20, Some(2))],
    );
    assert!(result.is_err());
    Ok(())
}
