use super::golden_comparison_regressions::{per_query_regressions, regressions};
use super::{
    AggregatedQualityMetrics, AggregatedResourceMetrics, AggregatedSecurityMetrics, BackendTier,
    GoldenComparisonReport, GoldenComparisonResult, GoldenCorpus, GoldenEvaluationReport,
    GoldenGate, GoldenGateConfig, GoldenGateError, GoldenObservation, GoldenProfile, Metric,
    PromotionDecision, calculate_report, validate_inputs,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PromotionRecord {
    pub evaluation_id: String,
    pub evaluation_date: String,
}

pub struct GoldenComparison {
    pub k: usize,
    pub tier: BackendTier,
    pub workload: String,
}

impl GoldenComparison {
    pub fn compare(
        &self,
        corpus: &GoldenCorpus,
        baseline_config: &GoldenGateConfig,
        baseline_observations: &[GoldenObservation],
        candidate_config: &GoldenGateConfig,
        candidate_observations: &[GoldenObservation],
        promotion_record: Option<PromotionRecord>,
    ) -> Result<GoldenComparisonResult, GoldenGateError> {
        if baseline_config.profile != GoldenProfile::V0_4
            || candidate_config.profile != GoldenProfile::V0_5
        {
            return Err(GoldenGateError::InvalidProfilePair);
        }
        if self.workload.trim().is_empty() {
            return Err(GoldenGateError::InvalidWorkload);
        }
        if self.k == 0 {
            return Err(GoldenGateError::InvalidK);
        }
        let baseline_reports =
            evaluate_reports(self.k, corpus, baseline_config, baseline_observations)?;
        let candidate_reports =
            evaluate_reports(self.k, corpus, candidate_config, candidate_observations)?;
        let baseline_quality = aggregate_quality(self.k, &baseline_reports);
        let candidate_quality = aggregate_quality(self.k, &candidate_reports);
        let baseline_resources = aggregate_resources(baseline_observations);
        let candidate_resources = aggregate_resources(candidate_observations);
        let baseline_security = aggregate_security(&baseline_reports);
        let candidate_security = aggregate_security(&candidate_reports);
        let report = GoldenComparisonReport {
            backend_tier: self.tier,
            workload: self.workload.clone(),
            corpus_snapshot: corpus.corpus_snapshot,
            index_generation: corpus.index_generation,
            fingerprint: corpus.fingerprint.clone(),
            baseline_profile: baseline_config.profile,
            candidate_profile: candidate_config.profile,
            baseline_quality: baseline_quality.clone(),
            candidate_quality: candidate_quality.clone(),
            baseline_resources: baseline_resources.clone(),
            candidate_resources: candidate_resources.clone(),
            baseline_security: baseline_security.clone(),
            candidate_security: candidate_security.clone(),
        };
        let decision = decide(
            DecisionContext {
                baseline_reports: &baseline_reports,
                candidate_reports: &candidate_reports,
                k: self.k,
                baseline_quality: &baseline_quality,
                candidate_quality: &candidate_quality,
                baseline_resources: &baseline_resources,
                candidate_resources: &candidate_resources,
                baseline_security: &baseline_security,
                candidate_security: &candidate_security,
                baseline_config,
                candidate_config,
            },
            promotion_record,
        )?;
        Ok(GoldenComparisonResult { report, decision })
    }
}

struct DecisionContext<'a> {
    baseline_reports: &'a [GoldenEvaluationReport],
    candidate_reports: &'a [GoldenEvaluationReport],
    k: usize,
    baseline_quality: &'a AggregatedQualityMetrics,
    candidate_quality: &'a AggregatedQualityMetrics,
    baseline_resources: &'a AggregatedResourceMetrics,
    candidate_resources: &'a AggregatedResourceMetrics,
    baseline_security: &'a AggregatedSecurityMetrics,
    candidate_security: &'a AggregatedSecurityMetrics,
    baseline_config: &'a GoldenGateConfig,
    candidate_config: &'a GoldenGateConfig,
}

fn decide(
    context: DecisionContext<'_>,
    promotion_record: Option<PromotionRecord>,
) -> Result<PromotionDecision, GoldenGateError> {
    let DecisionContext {
        baseline_reports,
        candidate_reports,
        k,
        baseline_quality,
        candidate_quality,
        baseline_resources,
        candidate_resources,
        baseline_security,
        candidate_security,
        baseline_config,
        candidate_config,
    } = context;
    let mut regressions = regressions(
        baseline_quality,
        candidate_quality,
        baseline_resources,
        candidate_resources,
        baseline_security,
        candidate_security,
    );
    regressions.extend(per_query_regressions(
        baseline_reports,
        candidate_reports,
        k,
    ));
    if baseline_resources.total_ingest_update_ms.is_some()
        != candidate_resources.total_ingest_update_ms.is_some()
    {
        regressions.push("total_ingest_update_ms availability differs".into());
    }
    if baseline_resources.total_energy_millijoules.is_some()
        != candidate_resources.total_energy_millijoules.is_some()
    {
        regressions.push("total_energy_millijoules availability differs".into());
    }
    let complete_telemetry = baseline_resources.total_ingest_update_ms.is_some()
        && candidate_resources.total_ingest_update_ms.is_some()
        && baseline_resources.total_energy_millijoules.is_some()
        && candidate_resources.total_energy_millijoules.is_some();
    let complete_budgets = baseline_config.max_ingest_update_ms.is_some()
        && candidate_config.max_ingest_update_ms.is_some()
        && baseline_config.max_energy_millijoules.is_some()
        && candidate_config.max_energy_millijoules.is_some();
    let material_delta = std::cmp::max(
        std::cmp::max(
            baseline_config.min_material_quality_delta.value(),
            candidate_config.min_material_quality_delta.value(),
        ),
        Metric::MATERIAL_QUALITY_DELTA.value(),
    );
    let materially_improves = |candidate: Metric, baseline: Metric| {
        candidate > baseline && candidate.value().saturating_sub(baseline.value()) >= material_delta
    };
    let improvement =
        materially_improves(
            candidate_quality.mean_recall_at_k,
            baseline_quality.mean_recall_at_k,
        ) || materially_improves(
            candidate_quality.mean_ndcg_at_k,
            baseline_quality.mean_ndcg_at_k,
        ) || materially_improves(candidate_quality.mean_mrr, baseline_quality.mean_mrr)
            || materially_improves(
                candidate_quality.mean_exact_span_recall,
                baseline_quality.mean_exact_span_recall,
            );
    if regressions.is_empty() && improvement && complete_telemetry && complete_budgets {
        let record = promotion_record.ok_or(GoldenGateError::MissingPromotionRecord)?;
        if record.evaluation_id.trim().is_empty() || record.evaluation_date.trim().is_empty() {
            return Err(GoldenGateError::MissingPromotionRecord);
        }
        Ok(PromotionDecision::Promote {
            evaluation_id: record.evaluation_id,
            evaluation_date: record.evaluation_date,
            reason: "Candidate materially improved quality without measured regressions".into(),
        })
    } else {
        let reason = if !regressions.is_empty() {
            format!("Candidate regressed on: {}", regressions.join(", "))
        } else if !complete_telemetry {
            "Promotion requires complete ingest/update and energy telemetry".into()
        } else if !complete_budgets {
            "Promotion requires explicit ingest/update and energy budgets".into()
        } else {
            "Candidate did not produce a material quality improvement".into()
        };
        Ok(PromotionDecision::RetainBaseline { reason })
    }
}

fn evaluate_reports(
    k: usize,
    corpus: &GoldenCorpus,
    config: &GoldenGateConfig,
    observations: &[GoldenObservation],
) -> Result<Vec<GoldenEvaluationReport>, GoldenGateError> {
    let gate = GoldenGate { config: *config, k };
    let indexed = validate_inputs(corpus, observations)?;
    if corpus.schema_version != GoldenGate::CURRENT_SCHEMA_VERSION {
        return Err(GoldenGateError::UnsupportedSchema(corpus.schema_version));
    }
    corpus
        .queries
        .iter()
        .map(|query| {
            let observation = indexed
                .get(&query.query_id)
                .ok_or(GoldenGateError::MissingObservation(query.query_id.value()))?;
            if observation.profile != config.profile {
                return Err(GoldenGateError::ProfileMismatch {
                    query_id: query.query_id.value(),
                    expected: config.profile,
                    found: observation.profile,
                });
            }
            let trace =
                observation
                    .outcome
                    .trace_data
                    .as_ref()
                    .ok_or(GoldenGateError::TraceMismatch {
                        query_id: query.query_id.value(),
                    })?;
            if observation.outcome.status != query.expected_status
                || observation.outcome.fingerprint != corpus.fingerprint
                || observation.outcome.index_generation != corpus.index_generation
                || trace.query_id != query.query_id
                || trace.original_query != query.original_query
                || trace.corpus_snapshot != corpus.corpus_snapshot
                || trace.index_generation != corpus.index_generation
                || trace.fingerprint != corpus.fingerprint
                || query.expected_plan.query_id != query.query_id
                || query.expected_plan.original_query != query.original_query
                || query.expected_plan.corpus_snapshot != corpus.corpus_snapshot
                || query.expected_plan.index_generation != corpus.index_generation
                || query.expected_plan.fingerprint != corpus.fingerprint
                || !trace.matches_plan(&query.expected_plan)
                || observation.outcome.trace != trace.deterministic_id()
                || !trace.matches_coverage(
                    &observation.outcome.coverage,
                    &observation.outcome.conflicts,
                    observation.outcome.evidence.len(),
                )
                || !trace.matches_evidence(&observation.outcome.evidence)
                || !trace.matches_outcome(
                    &observation.outcome.status,
                    observation.outcome.evidence.len(),
                )
            {
                return Err(GoldenGateError::TraceMismatch {
                    query_id: query.query_id.value(),
                });
            }
            let report = calculate_report(
                query,
                &observation.outcome,
                observation.resources,
                observation.security,
                k,
            );
            gate.check_thresholds(query.query_id.value(), &query.expected_status, &report)?;
            Ok(report)
        })
        .collect()
}

fn aggregate_quality(k: usize, reports: &[GoldenEvaluationReport]) -> AggregatedQualityMetrics {
    if reports.is_empty() {
        return AggregatedQualityMetrics::default();
    }
    let count = reports.len() as u64;
    let mean = |values: Vec<u32>| {
        (values
            .iter()
            .map(|value| u64::from(*value))
            .fold(0, u64::saturating_add)
            / count)
            .min(u64::from(u32::MAX)) as u32
    };
    AggregatedQualityMetrics {
        mean_recall_at_k: Metric::new(mean(
            reports
                .iter()
                .map(|report| {
                    report
                        .recall_at_k
                        .get(&k)
                        .map_or(0, |metric| metric.value())
                })
                .collect(),
        ))
        .map_or(Metric::ZERO, |metric| metric),
        mean_ndcg_at_k: Metric::new(mean(
            reports
                .iter()
                .map(|report| report.ndcg_at_k.get(&k).map_or(0, |metric| metric.value()))
                .collect(),
        ))
        .map_or(Metric::ZERO, |metric| metric),
        mean_mrr: Metric::new(mean(
            reports.iter().map(|report| report.mrr.value()).collect(),
        ))
        .map_or(Metric::ZERO, |metric| metric),
        mean_exact_span_recall: Metric::new(mean(
            reports
                .iter()
                .map(|report| report.exact_span_recall.value())
                .collect(),
        ))
        .map_or(Metric::ZERO, |metric| metric),
    }
}

fn aggregate_resources(observations: &[GoldenObservation]) -> AggregatedResourceMetrics {
    if observations.is_empty() {
        return AggregatedResourceMetrics::default();
    }
    let mut latencies = observations
        .iter()
        .map(|observation| observation.resources.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let percentile = |percentile: usize| {
        let rank = (latencies.len() * percentile).saturating_add(99) / 100;
        latencies[rank.saturating_sub(1)]
    };
    let total_energy = observations
        .iter()
        .map(|observation| observation.resources.energy_millijoules)
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().fold(0, u64::saturating_add));
    let total_ingest_update = observations
        .iter()
        .map(|observation| observation.resources.ingest_update_ms)
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().fold(0, u64::saturating_add));
    AggregatedResourceMetrics {
        p50_latency_ms: percentile(50),
        p95_latency_ms: percentile(95),
        p99_latency_ms: percentile(99),
        peak_memory_bytes: observations
            .iter()
            .map(|observation| observation.resources.memory_bytes)
            .fold(0, u64::max),
        peak_disk_bytes: observations
            .iter()
            .map(|observation| observation.resources.disk_bytes)
            .fold(0, u64::max),
        total_ingest_update_ms: total_ingest_update,
        total_energy_millijoules: total_energy,
    }
}

fn aggregate_security(reports: &[GoldenEvaluationReport]) -> AggregatedSecurityMetrics {
    AggregatedSecurityMetrics {
        total_acl_leakage: reports
            .iter()
            .map(|report| report.security.acl_leakage)
            .fold(0, u32::saturating_add),
        total_attack_successes: reports
            .iter()
            .map(|report| report.security.attack_successes)
            .fold(0, u32::saturating_add),
        total_privacy_violations: reports
            .iter()
            .map(|report| report.security.privacy_violations)
            .fold(0, u32::saturating_add),
    }
}
