//! Deterministic per-case quality scoring for the learned-sparse benchmark.
//!
//! The scorer reduces each retrieved candidate to the spans and judgment
//! grade a quality metric needs. The executor owns the mapping from domain
//! candidates to these inputs; the scorer owns the fixed, reproducible
//! formulas so every route and every case is scored identically.

use crate::golden::Metric;

use super::{
    CheckStatus, LearnedSparseAcceptedSpan, LearnedSparseBenchmarkError, LearnedSparseExpectedOutcome,
    LearnedSparseQualityMetrics, Measurement,
};

/// Character span of one retrieved candidate inside a corpus source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedSparseRetrievedSpan {
    pub source_id: String,
    pub start: u32,
    pub end: u32,
}

impl LearnedSparseRetrievedSpan {
    pub fn overlaps(&self, accepted: &LearnedSparseAcceptedSpan) -> bool {
        self.source_id == accepted.source_id
            && self.start < accepted.end
            && accepted.start < self.end
    }
}

/// One retrieved candidate reduced to the inputs the scorer consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedSparseRetrievedCandidate {
    pub evidence_id: String,
    /// One-based rank of the candidate in the route's result list.
    pub lane_rank: u32,
    pub span: LearnedSparseRetrievedSpan,
    /// Citation span when the route surfaced a citation for this candidate.
    pub citation: Option<LearnedSparseRetrievedSpan>,
    /// Corpus judgment grade for the candidate's source when judged
    /// (0 = NotRelevant, 1 = Relevant, 2 = HighlyRelevant).
    ///
    /// The three-level grades live in the task corpus judgments; the
    /// benchmark expected outcome carries only accepted spans, so the
    /// executor maps judgments onto candidates and the scorer stays pure.
    pub grade: Option<u8>,
}

fn ratio(numerator: usize, denominator: usize) -> Metric {
    Metric::from_ratio(numerator, denominator.max(1))
}

fn spans_covered(
    accepted: &[LearnedSparseAcceptedSpan],
    candidates: &[LearnedSparseRetrievedCandidate],
) -> Vec<bool> {
    accepted
        .iter()
        .map(|span| {
            candidates
                .iter()
                .any(|candidate| candidate.span.overlaps(span))
        })
        .collect()
}

fn dcg(grades: &[(u32, u8)]) -> f64 {
    grades
        .iter()
        .enumerate()
        .map(|(index, (_, grade))| {
            let gain = match grade {
                2 => 3.0_f64,
                1 => 1.0_f64,
                _ => 0.0_f64,
            };
            gain / (index as f64 + 2.0).log2()
        })
        .sum()
}

fn ndcg(candidates: &[LearnedSparseRetrievedCandidate], k: u32) -> Metric {
    let mut ranked = candidates
        .iter()
        .filter(|candidate| candidate.lane_rank <= k)
        .map(|candidate| (candidate.lane_rank, candidate.grade.unwrap_or(0)))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(rank, _)| *rank);
    let mut ideal = ranked
        .iter()
        .map(|(_, grade)| *grade)
        .collect::<Vec<_>>();
    ideal.sort_by(|left, right| right.cmp(left));
    let ideal = ideal
        .into_iter()
        .enumerate()
        .map(|(index, grade)| (index as u32 + 1, grade))
        .collect::<Vec<_>>();
    let ideal_dcg = dcg(&ideal);
    if ideal_dcg <= 0.0 {
        return Metric::ZERO;
    }
    let actual = ranked
        .into_iter()
        .enumerate()
        .map(|(index, (_, grade))| (index as u32 + 1, grade))
        .collect::<Vec<_>>();
    let value = dcg(&actual) / ideal_dcg;
    Metric::from_ratio(
        (value * f64::from(Metric::ONE.value())).round() as usize,
        Metric::ONE.value() as usize,
    )
}

fn mean_average_precision(
    accepted: &[LearnedSparseAcceptedSpan],
    candidates: &[LearnedSparseRetrievedCandidate],
) -> Metric {
    let mut ranked = candidates.to_vec();
    ranked.sort_by_key(|candidate| candidate.lane_rank);
    let relevant_total = accepted
        .iter()
        .filter(|span| ranked.iter().any(|candidate| candidate.span.overlaps(span)))
        .count();
    if relevant_total == 0 {
        return Metric::ZERO;
    }
    let mut covered = vec![false; accepted.len()];
    let mut relevant_seen = 0_usize;
    let mut precision_sum = 0.0_f64;
    for (index, candidate) in ranked.iter().enumerate() {
        let newly_covered = accepted
            .iter()
            .enumerate()
            .filter(|(span_index, span)| !covered[*span_index] && candidate.span.overlaps(span))
            .map(|(span_index, _)| span_index)
            .collect::<Vec<_>>();
        if !newly_covered.is_empty() {
            relevant_seen += 1;
            precision_sum += (relevant_seen as f64) / (index as f64 + 1.0);
            for span_index in newly_covered {
                covered[span_index] = true;
            }
        }
    }
    Metric::from_ratio(
        (precision_sum / relevant_total as f64 * f64::from(Metric::ONE.value())).round() as usize,
        Metric::ONE.value() as usize,
    )
}

/// Computes every quality metric field for one case on one route.
///
/// Candidates are expected in lane order; ranks are honored as given so the
/// executor controls what the route actually surfaced.
pub fn score_case(
    case_id: &str,
    expected: &LearnedSparseExpectedOutcome,
    candidates: &[LearnedSparseRetrievedCandidate],
) -> Result<LearnedSparseQualityMetrics, LearnedSparseBenchmarkError> {
    let invalid = candidates
        .iter()
        .find(|candidate| candidate.lane_rank == 0 || candidate.span.source_id.trim().is_empty());
    if let Some(candidate) = invalid {
        return Err(LearnedSparseBenchmarkError::InvalidMeasurement(format!(
            "case {case_id} candidate {} has an invalid lane rank or empty source",
            candidate.evidence_id
        )));
    }
    let mut ranked = candidates.to_vec();
    ranked.sort_by_key(|candidate| candidate.lane_rank);
    let answered = !ranked.is_empty();
    let expected_relevant = match expected {
        LearnedSparseExpectedOutcome::Evidence {
            accepted_spans,
            evidence_chain,
            ..
        } => {
            let accepted = accepted_spans;
            let covered = spans_covered(accepted, &ranked);
            let covered_count = covered.iter().filter(|value| **value).count();
            let recall = |k: u32| {
                let top = ranked
                    .iter()
                    .filter(|candidate| candidate.lane_rank <= k)
                    .cloned()
                    .collect::<Vec<_>>();
                let covered = spans_covered(accepted, &top);
                ratio(covered.iter().filter(|value| **value).count(), accepted.len())
            };
            let exact = ratio(covered_count, accepted.len());
            let chain_sources = evidence_chain
                .iter()
                .filter(|source| {
                    ranked
                        .iter()
                        .any(|candidate| &candidate.span.source_id == *source)
                })
                .count();
            let chain = ratio(chain_sources, evidence_chain.len());
            let expected_sources = accepted
                .iter()
                .map(|span| span.source_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let retrieved_sources = ranked
                .iter()
                .map(|candidate| candidate.span.source_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let relevant_sources = ranked
                .iter()
                .filter(|candidate| {
                    accepted
                        .iter()
                        .any(|span| candidate.span.overlaps(span))
                })
                .map(|candidate| candidate.span.source_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let diversity = ratio(
                relevant_sources.len().min(expected_sources.len()),
                expected_sources.len(),
            );
            let redundancy = if ranked.is_empty() {
                Metric::ZERO
            } else {
                let total = ranked.len();
                let distinct = retrieved_sources.len();
                ratio(total - distinct, total)
            };
            let citation_count = ranked
                .iter()
                .filter(|candidate| candidate.citation.is_some())
                .count();
            let citation_hits = ranked
                .iter()
                .filter(|candidate| {
                    candidate.citation.as_ref().is_some_and(|citation| {
                        accepted
                            .iter()
                            .any(|span| citation.overlaps(span))
                    })
                })
                .count();
            let citation_precision = ratio(citation_hits, citation_count);
            let citation_recall = ratio(
                accepted
                    .iter()
                    .filter(|span| {
                        ranked.iter().any(|candidate| {
                            candidate
                                .citation
                                .as_ref()
                                .is_some_and(|citation| citation.overlaps(span))
                        })
                    })
                    .count(),
                accepted.len(),
            );
            let mrr = ranked
                .iter()
                .find(|candidate| {
                    accepted
                        .iter()
                        .any(|span| candidate.span.overlaps(span))
                })
                .filter(|candidate| candidate.lane_rank <= 10)
                .map_or(Metric::ZERO, |candidate| {
                    ratio(1, candidate.lane_rank as usize)
                });
            let map = mean_average_precision(accepted, &ranked);
            let evidence = (
                recall(5),
                recall(20),
                recall(50),
                recall(100),
                ndcg(&ranked, 10),
                ndcg(&ranked, 20),
                mrr,
                map,
                exact,
                chain,
                diversity,
                redundancy,
                citation_precision,
                citation_recall,
            );
            let unsupported = if answered {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            };
            Some((evidence, unsupported, CheckStatus::NotDetected))
        }
        LearnedSparseExpectedOutcome::Abstain
        | LearnedSparseExpectedOutcome::UnsupportedClaim
        | LearnedSparseExpectedOutcome::Conflict => None,
    };

    let (quality, unsupported, conflict) = match expected {
        LearnedSparseExpectedOutcome::Evidence { .. } => {
            let (evidence, unsupported, conflict) =
                expected_relevant.ok_or_else(|| {
                    LearnedSparseBenchmarkError::InvalidMeasurement(format!(
                        "case {case_id} evidence expectation produced no quality profile"
                    ))
                })?;
            (evidence, unsupported, conflict)
        }
        LearnedSparseExpectedOutcome::UnsupportedClaim => {
            let status = if answered {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            (
                zero_evidence_metrics(),
                status,
                CheckStatus::NotDetected,
            )
        }
        LearnedSparseExpectedOutcome::Conflict => {
            let status = if answered {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            (
                zero_evidence_metrics(),
                CheckStatus::NotDetected,
                status,
            )
        }
        LearnedSparseExpectedOutcome::Abstain => (
            zero_evidence_metrics(),
            CheckStatus::NotDetected,
            CheckStatus::NotDetected,
        ),
    };

    let expects_abstention = matches!(
        expected,
        LearnedSparseExpectedOutcome::Abstain
            | LearnedSparseExpectedOutcome::UnsupportedClaim
            | LearnedSparseExpectedOutcome::Conflict
    );
    let abstention_ok = if expects_abstention {
        !answered
    } else {
        answered
    };
    let abstention_precision = if abstention_ok {
        Metric::ONE
    } else {
        Metric::ZERO
    };
    let abstention_recall = if abstention_ok {
        Metric::ONE
    } else {
        Metric::ZERO
    };

    let measured = |value: Metric| Measurement::measured(value);
    let status = |value: CheckStatus| Measurement::measured(value);
    let (
        recall_at_5,
        recall_at_20,
        recall_at_50,
        recall_at_100,
        ndcg_at_10,
        ndcg_at_20,
        mrr_at_10,
        mean_average_precision,
        exact_span_recall,
        evidence_chain_coverage,
        source_diversity,
        source_redundancy,
        citation_precision,
        citation_recall,
    ) = quality;
    let metrics = LearnedSparseQualityMetrics {
        recall_at_5: measured(recall_at_5),
        recall_at_20: measured(recall_at_20),
        recall_at_50: measured(recall_at_50),
        recall_at_100: measured(recall_at_100),
        ndcg_at_10: measured(ndcg_at_10),
        ndcg_at_20: measured(ndcg_at_20),
        mrr_at_10: measured(mrr_at_10),
        mean_average_precision: measured(mean_average_precision),
        exact_span_recall: measured(exact_span_recall),
        evidence_chain_coverage: measured(evidence_chain_coverage),
        source_diversity: measured(source_diversity),
        source_redundancy: measured(source_redundancy),
        citation_precision: measured(citation_precision),
        citation_recall: measured(citation_recall),
        abstention_precision: measured(abstention_precision),
        abstention_recall: measured(abstention_recall),
        unsupported_claim_status: status(unsupported),
        conflict_detection_status: status(conflict),
    };
    metrics.validate()?;
    Ok(metrics)
}

fn zero_evidence_metrics() -> (
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
    Metric,
) {
    (
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
        Metric::ZERO,
    )
}

#[cfg(test)]
mod tests {
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
    fn empty_route_on_evidence_case_scores_zero_and_fails_checks() -> Result<(), Box<dyn std::error::Error>> {
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
    fn abstain_case_rewards_abstention_and_penalizes_answers() -> Result<(), Box<dyn std::error::Error>> {
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
            abstained
                .unsupported_claim_status
                .measured_value(),
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
        let recall = metrics.exact_span_recall.measured_value().copied().unwrap();
        assert_eq!(recall.value(), Metric::ONE.value() / 3);
        let mrr = metrics.mrr_at_10.measured_value().copied().unwrap();
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
}
