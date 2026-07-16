use super::*;
use std::collections::{BTreeMap, BTreeSet};

impl GoldenGate {
    pub const CURRENT_SCHEMA_VERSION: u32 = 2;

    pub fn evaluate(
        &self,
        corpus: &GoldenCorpus,
        observations: &[GoldenObservation],
    ) -> Result<Vec<GoldenEvaluationReport>, GoldenGateError> {
        if self.k == 0 {
            return Err(GoldenGateError::InvalidK);
        }
        if corpus.schema_version != Self::CURRENT_SCHEMA_VERSION {
            return Err(GoldenGateError::UnsupportedSchema(corpus.schema_version));
        }
        let indexed = validate_inputs(corpus, observations)?;
        let mut reports = Vec::with_capacity(corpus.queries.len());
        for query in &corpus.queries {
            let observation = indexed
                .get(&query.query_id)
                .ok_or(GoldenGateError::MissingObservation(query.query_id.value()))?;
            if observation.profile != self.config.profile {
                return Err(GoldenGateError::ProfileMismatch {
                    query_id: query.query_id.value(),
                    expected: self.config.profile,
                    found: observation.profile,
                });
            }
            if observation.outcome.status != query.expected_status {
                return Err(GoldenGateError::StatusMismatch {
                    query_id: query.query_id.value(),
                    expected: query.expected_status.clone(),
                    found: observation.outcome.status.clone(),
                });
            }
            let Some(trace) = observation.outcome.trace_data.as_ref() else {
                return Err(GoldenGateError::TraceMismatch {
                    query_id: query.query_id.value(),
                });
            };
            if observation.outcome.fingerprint != corpus.fingerprint
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
                self.k,
            );
            self.check_thresholds(query.query_id.value(), &query.expected_status, &report)?;
            reports.push(report);
        }
        Ok(reports)
    }

    pub(crate) fn check_thresholds(
        &self,
        query_id: u64,
        expected_status: &maestria_domain::SearchStatus,
        report: &GoldenEvaluationReport,
    ) -> Result<(), GoldenGateError> {
        let empty_expected = matches!(
            expected_status,
            maestria_domain::SearchStatus::NoEvidenceFound
                | maestria_domain::SearchStatus::Abstained
                | maestria_domain::SearchStatus::DeniedByPolicy
                | maestria_domain::SearchStatus::QuarantinedForReview
        );
        if !empty_expected {
            let recall = report
                .recall_at_k
                .get(&self.k)
                .copied()
                .map_or(Metric::ZERO, |value| value);
            let ndcg = report
                .ndcg_at_k
                .get(&self.k)
                .copied()
                .map_or(Metric::ZERO, |value| value);
            let checks = [
                (recall < self.config.min_recall_at_k, "Recall@k"),
                (ndcg < self.config.min_ndcg_at_k, "nDCG@k"),
                (report.mrr < self.config.min_mrr, "MRR"),
                (
                    report.exact_span_recall < self.config.min_exact_span_recall,
                    "exact-span recall",
                ),
            ];
            if let Some((true, name)) = checks.into_iter().find(|(failed, _)| *failed) {
                return Err(GoldenGateError::Regression {
                    query_id,
                    reason: name.to_string(),
                });
            }
        }
        if !report.resources.telemetry_complete {
            return Err(regression(query_id, "resource telemetry"));
        }
        if !report.security.telemetry_complete {
            return Err(regression(query_id, "security telemetry"));
        }
        if report.resources.latency_ms > self.config.max_latency_ms {
            return Err(regression(query_id, "latency"));
        }
        if report.resources.memory_bytes > self.config.max_memory_bytes {
            return Err(regression(query_id, "memory"));
        }
        if report.resources.disk_bytes > self.config.max_disk_bytes {
            return Err(regression(query_id, "disk"));
        }
        if let (Some(value), Some(maximum)) = (
            report.resources.ingest_update_ms,
            self.config.max_ingest_update_ms,
        ) && value > maximum
        {
            return Err(regression(query_id, "ingest/update cost"));
        }
        if let (Some(value), Some(maximum)) = (
            report.resources.energy_millijoules,
            self.config.max_energy_millijoules,
        ) && value > maximum
        {
            return Err(regression(query_id, "energy"));
        }
        if report.security.acl_leakage > self.config.max_acl_leakage {
            return Err(regression(query_id, "ACL leakage"));
        }
        if report.security.attack_successes > self.config.max_attack_successes {
            return Err(regression(query_id, "attack success"));
        }
        if report.security.privacy_violations > self.config.max_privacy_violations {
            return Err(regression(query_id, "privacy violations"));
        }
        Ok(())
    }
}

pub(crate) fn validate_inputs<'a>(
    corpus: &GoldenCorpus,
    observations: &'a [GoldenObservation],
) -> Result<BTreeMap<maestria_domain::QueryId, &'a GoldenObservation>, GoldenGateError> {
    if corpus.queries.is_empty() {
        return Err(GoldenGateError::EmptyCorpus);
    }
    let mut indexed = BTreeMap::new();
    for observation in observations {
        if indexed.insert(observation.query_id, observation).is_some() {
            return Err(GoldenGateError::DuplicateObservation(
                observation.query_id.value(),
            ));
        }
    }
    let mut expected_queries = BTreeSet::new();
    for query in &corpus.queries {
        if !expected_queries.insert(query.query_id) {
            return Err(GoldenGateError::DuplicateQuery(query.query_id.value()));
        }
        let allows_empty = matches!(
            &query.expected_status,
            maestria_domain::SearchStatus::NoEvidenceFound
                | maestria_domain::SearchStatus::Abstained
                | maestria_domain::SearchStatus::DeniedByPolicy
                | maestria_domain::SearchStatus::QuarantinedForReview
        );
        if query.judgments.is_empty() && !allows_empty {
            return Err(GoldenGateError::EmptyJudgments(query.query_id.value()));
        }
        if !query
            .judgments
            .iter()
            .any(|judgment| judgment.relevance > 0)
            && !allows_empty
        {
            return Err(GoldenGateError::NoRelevantJudgments(query.query_id.value()));
        }
        let mut judgments = BTreeSet::new();
        for judgment in &query.judgments {
            if !judgments.insert(judgment.evidence_id) {
                return Err(GoldenGateError::DuplicateJudgment {
                    query_id: query.query_id.value(),
                    evidence_id: judgment.evidence_id.value(),
                });
            }
        }
    }
    if let Some(observation) = observations
        .iter()
        .find(|observation| !expected_queries.contains(&observation.query_id))
    {
        return Err(GoldenGateError::UnexpectedObservation(
            observation.query_id.value(),
        ));
    }
    Ok(indexed)
}

fn regression(query_id: u64, reason: &str) -> GoldenGateError {
    GoldenGateError::Regression {
        query_id,
        reason: reason.to_string(),
    }
}
