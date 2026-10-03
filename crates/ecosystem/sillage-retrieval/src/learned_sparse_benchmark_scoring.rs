//! Deterministic per-case quality scoring for the learned-sparse benchmark.
//!
//! The scorer reduces each retrieved candidate to the spans and judgment
//! grade a quality metric needs. The executor owns the mapping from domain
//! candidates to these inputs; the scorer owns the fixed, reproducible
//! formulas so every route and every case is scored identically.

use crate::golden::Metric;

use super::{
    CheckStatus, LearnedSparseAcceptedSpan, LearnedSparseBenchmarkError,
    LearnedSparseExpectedOutcome, LearnedSparseQualityMetrics, Measurement,
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
        .map(|candidate| {
            let grade = candidate.grade.map_or(0, u8::from);
            (candidate.lane_rank, grade)
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(rank, _)| *rank);
    let mut ideal = ranked.iter().map(|(_, grade)| *grade).collect::<Vec<_>>();
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

/// The evidence-quality metric block for an expected-evidence case.
struct EvidenceQuality {
    recall_at_5: Metric,
    recall_at_20: Metric,
    recall_at_50: Metric,
    recall_at_100: Metric,
    ndcg_at_10: Metric,
    ndcg_at_20: Metric,
    mrr_at_10: Metric,
    mean_average_precision: Metric,
    exact_span_recall: Metric,
    evidence_chain_coverage: Metric,
    source_diversity: Metric,
    source_redundancy: Metric,
    citation_precision: Metric,
    citation_recall: Metric,
}

fn evidence_quality(
    accepted: &[LearnedSparseAcceptedSpan],
    evidence_chain: &[String],
    ranked: &[LearnedSparseRetrievedCandidate],
) -> EvidenceQuality {
    let covered = spans_covered(accepted, ranked);
    let covered_count = covered.iter().filter(|value| **value).count();
    let recall = |k: u32| {
        let top = ranked
            .iter()
            .filter(|candidate| candidate.lane_rank <= k)
            .cloned()
            .collect::<Vec<_>>();
        let covered = spans_covered(accepted, &top);
        ratio(
            covered.iter().filter(|value| **value).count(),
            accepted.len(),
        )
    };
    let chain_sources = evidence_chain
        .iter()
        .filter(|source| {
            ranked
                .iter()
                .any(|candidate| &candidate.span.source_id == *source)
        })
        .count();
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
        .filter(|candidate| accepted.iter().any(|span| candidate.span.overlaps(span)))
        .map(|candidate| candidate.span.source_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let citation_count = ranked
        .iter()
        .filter(|candidate| candidate.citation.is_some())
        .count();
    let citation_hits = ranked
        .iter()
        .filter(|candidate| {
            candidate
                .citation
                .as_ref()
                .is_some_and(|citation| accepted.iter().any(|span| citation.overlaps(span)))
        })
        .count();
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
        .find(|candidate| accepted.iter().any(|span| candidate.span.overlaps(span)))
        .filter(|candidate| candidate.lane_rank <= 10)
        .map(|candidate| ratio(1, candidate.lane_rank as usize));
    let redundancy = if ranked.is_empty() {
        Metric::ZERO
    } else {
        ratio(ranked.len() - retrieved_sources.len(), ranked.len())
    };
    EvidenceQuality {
        recall_at_5: recall(5),
        recall_at_20: recall(20),
        recall_at_50: recall(50),
        recall_at_100: recall(100),
        ndcg_at_10: ndcg(ranked, 10),
        ndcg_at_20: ndcg(ranked, 20),
        mrr_at_10: match mrr {
            Some(mrr) => mrr,
            None => Metric::ZERO,
        },
        mean_average_precision: mean_average_precision(accepted, ranked),
        exact_span_recall: ratio(covered_count, accepted.len()),
        evidence_chain_coverage: ratio(chain_sources, evidence_chain.len()),
        source_diversity: ratio(
            relevant_sources.len().min(expected_sources.len()),
            expected_sources.len(),
        ),
        source_redundancy: redundancy,
        citation_precision: ratio(citation_hits, citation_count),
        citation_recall,
    }
}

fn zero_evidence_quality() -> EvidenceQuality {
    EvidenceQuality {
        recall_at_5: Metric::ZERO,
        recall_at_20: Metric::ZERO,
        recall_at_50: Metric::ZERO,
        recall_at_100: Metric::ZERO,
        ndcg_at_10: Metric::ZERO,
        ndcg_at_20: Metric::ZERO,
        mrr_at_10: Metric::ZERO,
        mean_average_precision: Metric::ZERO,
        exact_span_recall: Metric::ZERO,
        evidence_chain_coverage: Metric::ZERO,
        source_diversity: Metric::ZERO,
        source_redundancy: Metric::ZERO,
        citation_precision: Metric::ZERO,
        citation_recall: Metric::ZERO,
    }
}

/// The abstention block: whether the route answered when it should and
/// abstained when it should.
fn abstention_metrics(expected: &LearnedSparseExpectedOutcome, answered: bool) -> (Metric, Metric) {
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
    let value = if abstention_ok {
        Metric::ONE
    } else {
        Metric::ZERO
    };
    (value, value)
}

/// The check-status block for unsupported claims and conflicts.
fn outcome_checks(
    expected: &LearnedSparseExpectedOutcome,
    answered: bool,
) -> (CheckStatus, CheckStatus) {
    match expected {
        LearnedSparseExpectedOutcome::UnsupportedClaim => {
            let status = if answered {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            (status, CheckStatus::NotDetected)
        }
        LearnedSparseExpectedOutcome::Conflict => {
            let status = if answered {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            (CheckStatus::NotDetected, status)
        }
        LearnedSparseExpectedOutcome::Evidence { .. } => {
            let status = if answered {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            };
            (status, CheckStatus::NotDetected)
        }
        LearnedSparseExpectedOutcome::Abstain => {
            (CheckStatus::NotDetected, CheckStatus::NotDetected)
        }
    }
}

/// Validates the candidate list shape before scoring.
fn validate_candidates(
    case_id: &str,
    candidates: &[LearnedSparseRetrievedCandidate],
) -> Result<(), LearnedSparseBenchmarkError> {
    let invalid = candidates
        .iter()
        .find(|candidate| candidate.lane_rank == 0 || candidate.span.source_id.trim().is_empty());
    match invalid {
        Some(candidate) => Err(LearnedSparseBenchmarkError::InvalidMeasurement(format!(
            "case {case_id} candidate {} has an invalid lane rank or empty source",
            candidate.evidence_id
        ))),
        None => Ok(()),
    }
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
    validate_candidates(case_id, candidates)?;
    let mut ranked = candidates.to_vec();
    ranked.sort_by_key(|candidate| candidate.lane_rank);
    let answered = !ranked.is_empty();
    let (abstention_precision, abstention_recall) = abstention_metrics(expected, answered);
    let (unsupported, conflict) = outcome_checks(expected, answered);
    let quality = match expected {
        LearnedSparseExpectedOutcome::Evidence {
            accepted_spans,
            evidence_chain,
            ..
        } => evidence_quality(accepted_spans, evidence_chain, &ranked),
        _ => zero_evidence_quality(),
    };
    let metrics = LearnedSparseQualityMetrics {
        recall_at_5: Measurement::measured(quality.recall_at_5),
        recall_at_20: Measurement::measured(quality.recall_at_20),
        recall_at_50: Measurement::measured(quality.recall_at_50),
        recall_at_100: Measurement::measured(quality.recall_at_100),
        ndcg_at_10: Measurement::measured(quality.ndcg_at_10),
        ndcg_at_20: Measurement::measured(quality.ndcg_at_20),
        mrr_at_10: Measurement::measured(quality.mrr_at_10),
        mean_average_precision: Measurement::measured(quality.mean_average_precision),
        exact_span_recall: Measurement::measured(quality.exact_span_recall),
        evidence_chain_coverage: Measurement::measured(quality.evidence_chain_coverage),
        source_diversity: Measurement::measured(quality.source_diversity),
        source_redundancy: Measurement::measured(quality.source_redundancy),
        citation_precision: Measurement::measured(quality.citation_precision),
        citation_recall: Measurement::measured(quality.citation_recall),
        abstention_precision: Measurement::measured(abstention_precision),
        abstention_recall: Measurement::measured(abstention_recall),
        unsupported_claim_status: Measurement::measured(unsupported),
        conflict_detection_status: Measurement::measured(conflict),
    };
    metrics.validate()?;
    Ok(metrics)
}

#[cfg(test)]
#[path = "learned_sparse_benchmark_scoring_tests.rs"]
mod tests;
