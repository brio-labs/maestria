use super::{
    LearnedSparseBenchmarkObservation, LearnedSparseQualityMetrics, LearnedSparseResourceMetrics,
};

pub(super) fn aggregate_quality(
    selected: &[&LearnedSparseBenchmarkObservation],
) -> LearnedSparseQualityMetrics {
    LearnedSparseQualityMetrics {
        recall_at_5: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.recall_at_5).collect(),
        ),
        recall_at_20: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.recall_at_20).collect(),
        ),
        recall_at_50: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.recall_at_50).collect(),
        ),
        recall_at_100: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.recall_at_100).collect(),
        ),
        ndcg_at_10: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.ndcg_at_10).collect(),
        ),
        ndcg_at_20: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.ndcg_at_20).collect(),
        ),
        mrr_at_10: super::metrics::aggregate_metric(
            selected.iter().map(|o| &o.quality.mrr_at_10).collect(),
        ),
        mean_average_precision: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.mean_average_precision)
                .collect(),
        ),
        exact_span_recall: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.exact_span_recall)
                .collect(),
        ),
        evidence_chain_coverage: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.evidence_chain_coverage)
                .collect(),
        ),
        source_diversity: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.source_diversity)
                .collect(),
        ),
        source_redundancy: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.source_redundancy)
                .collect(),
        ),
        citation_precision: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.citation_precision)
                .collect(),
        ),
        citation_recall: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.citation_recall)
                .collect(),
        ),
        abstention_precision: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.abstention_precision)
                .collect(),
        ),
        abstention_recall: super::metrics::aggregate_metric(
            selected
                .iter()
                .map(|o| &o.quality.abstention_recall)
                .collect(),
        ),
        unsupported_claim_status: super::metrics::aggregate_check(
            selected
                .iter()
                .map(|o| &o.quality.unsupported_claim_status)
                .collect(),
        ),
        conflict_detection_status: super::metrics::aggregate_check(
            selected
                .iter()
                .map(|o| &o.quality.conflict_detection_status)
                .collect(),
        ),
    }
}

pub(super) fn aggregate_resources(
    selected: &[&LearnedSparseBenchmarkObservation],
) -> LearnedSparseResourceMetrics {
    LearnedSparseResourceMetrics {
        p50_latency_ms: super::metrics::aggregate_percentile(
            selected
                .iter()
                .map(|o| &o.resources.p50_latency_ms)
                .collect(),
            50,
        ),
        p95_latency_ms: super::metrics::aggregate_percentile(
            selected
                .iter()
                .map(|o| &o.resources.p95_latency_ms)
                .collect(),
            95,
        ),
        p99_latency_ms: super::metrics::aggregate_percentile(
            selected
                .iter()
                .map(|o| &o.resources.p99_latency_ms)
                .collect(),
            99,
        ),
        peak_ram_bytes: super::metrics::aggregate_max(
            selected
                .iter()
                .map(|o| &o.resources.peak_ram_bytes)
                .collect(),
        ),
        index_disk_bytes: super::metrics::aggregate_max(
            selected
                .iter()
                .map(|o| &o.resources.index_disk_bytes)
                .collect(),
        ),
        initial_indexing: super::metrics::aggregate_operation(selected, |resources| {
            &resources.initial_indexing
        }),
        incremental_update: super::metrics::aggregate_operation(selected, |resources| {
            &resources.incremental_update
        }),
        deletion: super::metrics::aggregate_operation(selected, |resources| &resources.deletion),
        rebuild: super::metrics::aggregate_operation(selected, |resources| &resources.rebuild),
        activation: super::metrics::aggregate_operation(selected, |resources| {
            &resources.activation
        }),
        rollback: super::metrics::aggregate_operation(selected, |resources| &resources.rollback),
    }
}
