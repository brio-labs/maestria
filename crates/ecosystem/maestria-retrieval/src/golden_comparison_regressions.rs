use super::{GoldenEvaluationReport, Metric};

pub(super) fn per_query_regressions(
    baseline_reports: &[GoldenEvaluationReport],
    candidate_reports: &[GoldenEvaluationReport],
    k: usize,
) -> Vec<String> {
    let mut regressions = Vec::new();
    for (baseline, candidate) in baseline_reports.iter().zip(candidate_reports) {
        if baseline.query_id != candidate.query_id {
            regressions.push(format!(
                "query identity {} != {}",
                baseline.query_id.value(),
                candidate.query_id.value()
            ));
            continue;
        }
        let baseline_recall = baseline
            .recall_at_k
            .get(&k)
            .copied()
            .map_or(Metric::ZERO, |value| value);
        let candidate_recall = candidate
            .recall_at_k
            .get(&k)
            .copied()
            .map_or(Metric::ZERO, |value| value);
        if candidate_recall < baseline_recall {
            regressions.push(format!("query {} recall_at_k", baseline.query_id.value()));
        }
        let baseline_ndcg = baseline
            .ndcg_at_k
            .get(&k)
            .copied()
            .map_or(Metric::ZERO, |value| value);
        let candidate_ndcg = candidate
            .ndcg_at_k
            .get(&k)
            .copied()
            .map_or(Metric::ZERO, |value| value);
        if candidate_ndcg < baseline_ndcg {
            regressions.push(format!("query {} ndcg_at_k", baseline.query_id.value()));
        }
        if candidate.mrr < baseline.mrr {
            regressions.push(format!("query {} mrr", baseline.query_id.value()));
        }
        if candidate.exact_span_recall < baseline.exact_span_recall {
            regressions.push(format!(
                "query {} exact_span_recall",
                baseline.query_id.value()
            ));
        }
        if candidate.resources.latency_ms > baseline.resources.latency_ms {
            regressions.push(format!("query {} latency_ms", baseline.query_id.value()));
        }
        if candidate.resources.memory_bytes > baseline.resources.memory_bytes {
            regressions.push(format!("query {} memory_bytes", baseline.query_id.value()));
        }
        if candidate.resources.disk_bytes > baseline.resources.disk_bytes {
            regressions.push(format!("query {} disk_bytes", baseline.query_id.value()));
        }
        if candidate.resources.ingest_update_ms > baseline.resources.ingest_update_ms {
            regressions.push(format!(
                "query {} ingest_update_ms",
                baseline.query_id.value()
            ));
        }
        if candidate.resources.energy_millijoules > baseline.resources.energy_millijoules {
            regressions.push(format!(
                "query {} energy_millijoules",
                baseline.query_id.value()
            ));
        }
        if candidate.security.acl_leakage > baseline.security.acl_leakage {
            regressions.push(format!("query {} acl_leakage", baseline.query_id.value()));
        }
        if candidate.security.attack_successes > baseline.security.attack_successes {
            regressions.push(format!(
                "query {} attack_successes",
                baseline.query_id.value()
            ));
        }
        if candidate.security.privacy_violations > baseline.security.privacy_violations {
            regressions.push(format!(
                "query {} privacy_violations",
                baseline.query_id.value()
            ));
        }
    }
    if baseline_reports.len() != candidate_reports.len() {
        regressions.push("query count differs".into());
    }
    regressions
}

pub(super) fn regressions(
    baseline_quality: &super::AggregatedQualityMetrics,
    candidate_quality: &super::AggregatedQualityMetrics,
    baseline_resources: &super::AggregatedResourceMetrics,
    candidate_resources: &super::AggregatedResourceMetrics,
    baseline_security: &super::AggregatedSecurityMetrics,
    candidate_security: &super::AggregatedSecurityMetrics,
) -> Vec<String> {
    let mut regressions = Vec::new();
    if candidate_quality.mean_recall_at_k < baseline_quality.mean_recall_at_k {
        regressions.push("mean_recall_at_k".into());
    }
    if candidate_quality.mean_ndcg_at_k < baseline_quality.mean_ndcg_at_k {
        regressions.push("mean_ndcg_at_k".into());
    }
    if candidate_quality.mean_mrr < baseline_quality.mean_mrr {
        regressions.push("mean_mrr".into());
    }
    if candidate_quality.mean_exact_span_recall < baseline_quality.mean_exact_span_recall {
        regressions.push("mean_exact_span_recall".into());
    }
    if candidate_resources.p50_latency_ms > baseline_resources.p50_latency_ms {
        regressions.push("p50_latency_ms".into());
    }
    if candidate_resources.p95_latency_ms > baseline_resources.p95_latency_ms {
        regressions.push("p95_latency_ms".into());
    }
    if candidate_resources.p99_latency_ms > baseline_resources.p99_latency_ms {
        regressions.push("p99_latency_ms".into());
    }
    if candidate_resources.peak_memory_bytes > baseline_resources.peak_memory_bytes {
        regressions.push("peak_memory_bytes".into());
    }
    if candidate_resources.peak_disk_bytes > baseline_resources.peak_disk_bytes {
        regressions.push("peak_disk_bytes".into());
    }
    if candidate_resources.total_ingest_update_ms > baseline_resources.total_ingest_update_ms {
        regressions.push("total_ingest_update_ms".into());
    }
    if candidate_resources.total_energy_millijoules > baseline_resources.total_energy_millijoules {
        regressions.push("total_energy_millijoules".into());
    }
    if candidate_security.total_acl_leakage > baseline_security.total_acl_leakage {
        regressions.push("total_acl_leakage".into());
    }
    if candidate_security.total_attack_successes > baseline_security.total_attack_successes {
        regressions.push("total_attack_successes".into());
    }
    if candidate_security.total_privacy_violations > baseline_security.total_privacy_violations {
        regressions.push("total_privacy_violations".into());
    }
    regressions
}
