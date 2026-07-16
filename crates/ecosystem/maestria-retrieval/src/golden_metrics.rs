use super::{
    GoldenEvaluationReport, GoldenGate, GoldenQuery, Metric, ResourceMetrics, SecurityMetrics,
};
use maestria_domain::{EvidenceId, SearchOutcome};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn calculate_report(
    query: &GoldenQuery,
    outcome: &SearchOutcome,
    resources: ResourceMetrics,
    security: SecurityMetrics,
    k: usize,
) -> GoldenEvaluationReport {
    let relevance = query
        .judgments
        .iter()
        .map(|judgment| (judgment.evidence_id, judgment.relevance))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let ranked = outcome
        .evidence
        .iter()
        .filter(|candidate| seen.insert(candidate.evidence_id))
        .collect::<Vec<_>>();
    let top = ranked.iter().take(k).copied().collect::<Vec<_>>();
    let relevant_total = relevance.values().filter(|score| **score > 0).count();
    let relevant_found = top
        .iter()
        .filter(|candidate| {
            relevance
                .get(&candidate.evidence_id)
                .is_some_and(|score| *score > 0)
        })
        .count();
    let recall = Metric::from_ratio(relevant_found, relevant_total);
    let mrr = ranked
        .iter()
        .position(|candidate| {
            relevance
                .get(&candidate.evidence_id)
                .is_some_and(|score| *score > 0)
        })
        .map_or(Metric::ZERO, |rank| Metric::from_ratio(1, rank + 1));

    let mut ndcg_at_k = BTreeMap::new();
    let mut recall_at_k = BTreeMap::new();
    recall_at_k.insert(k, recall);
    ndcg_at_k.insert(k, ndcg(&relevance, &top, k));

    let exact_expected = query
        .judgments
        .iter()
        .filter(|judgment| judgment.exact_span.is_some())
        .count();
    let exact_found = query
        .judgments
        .iter()
        .filter(|judgment| {
            judgment.exact_span.as_ref().is_some_and(|span| {
                outcome.evidence.iter().any(|candidate| {
                    candidate.evidence_id == judgment.evidence_id && &candidate.source_span == span
                })
            })
        })
        .count();

    GoldenEvaluationReport {
        schema_version: GoldenGate::CURRENT_SCHEMA_VERSION,
        query_id: query.query_id,
        recall_at_k,
        ndcg_at_k,
        mrr,
        exact_span_recall: Metric::from_ratio(exact_found, exact_expected),
        resources,
        security,
    }
}

fn ndcg(
    relevance: &BTreeMap<EvidenceId, u8>,
    top: &[&maestria_domain::EvidenceCandidate],
    k: usize,
) -> Metric {
    let dcg = top
        .iter()
        .enumerate()
        .map(|(rank, candidate)| {
            let score = relevance
                .get(&candidate.evidence_id)
                .copied()
                .map_or(0, |value| value);
            (2f64.powi(i32::from(score)) - 1.0) / ((rank + 2) as f64).log2()
        })
        .sum::<f64>();
    let mut ideal = relevance.values().copied().collect::<Vec<_>>();
    ideal.sort_unstable_by_key(|score| std::cmp::Reverse(*score));
    let idcg = ideal
        .into_iter()
        .take(k)
        .enumerate()
        .map(|(rank, score)| (2f64.powi(i32::from(score)) - 1.0) / ((rank + 2) as f64).log2())
        .sum::<f64>();
    if idcg == 0.0 {
        Metric::ONE
    } else {
        Metric::from_unit_interval(dcg / idcg)
    }
}
