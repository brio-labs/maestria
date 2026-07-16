use maestria_domain::{EvidenceId, EvidenceSpan, SearchOutcome};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
#[path = "golden_metrics.rs"]
mod golden_metrics;
use golden_metrics::calculate_report;

/// Fixed-point metric in the inclusive range 0..=10_000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Metric(u32);

impl Metric {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(10_000);

    pub fn new(value: u32) -> Option<Self> {
        (value <= Self::ONE.0).then_some(Self(value))
    }

    pub fn from_ratio(numerator: usize, denominator: usize) -> Self {
        if denominator == 0 {
            return Self::ONE;
        }
        let scaled = numerator.saturating_mul(Self::ONE.0 as usize) / denominator;
        Self(scaled.min(Self::ONE.0 as usize) as u32)
    }

    pub fn value(self) -> u32 {
        self.0
    }
    fn from_unit_interval(value: f64) -> Self {
        if value <= 0.0 {
            return Self::ZERO;
        }
        if value >= 1.0 {
            return Self::ONE;
        }
        Self((value * f64::from(Self::ONE.0)).round() as u32)
    }
}

/// Versioned judged corpus used by deterministic retrieval gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenCorpus {
    pub schema_version: u32,
    pub corpus_snapshot: maestria_domain::CorpusSnapshotId,
    pub index_generation: maestria_domain::IndexGenerationId,
    pub fingerprint: maestria_domain::RetrievalModelFingerprint,
    pub queries: Vec<GoldenQuery>,
}

/// One frozen query and its source-grounded judgments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenQuery {
    pub query_id: maestria_domain::QueryId,
    pub original_query: String,
    pub expected_plan: maestria_domain::SearchPlan,
    pub expected_status: maestria_domain::SearchStatus,
    pub judgments: Vec<GoldenJudgment>,
}

/// Relevance and exact-span judgment for one evidence item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenJudgment {
    pub evidence_id: EvidenceId,
    pub relevance: u8,
    pub exact_span: Option<EvidenceSpan>,
}

/// Measurements supplied by the harness around one search execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub latency_ms: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
}

/// Security measurements supplied by retrieval validators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityMetrics {
    pub acl_leakage: u32,
    pub attack_successes: u32,
}

/// Deterministic ranking and operational measurements for one query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenEvaluationReport {
    pub schema_version: u32,
    pub query_id: maestria_domain::QueryId,
    pub recall_at_k: BTreeMap<usize, Metric>,
    pub ndcg_at_k: BTreeMap<usize, Metric>,
    pub mrr: Metric,
    pub exact_span_recall: Metric,
    pub resources: ResourceMetrics,
    pub security: SecurityMetrics,
}

/// A query result plus measurements captured by the evaluation harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenObservation {
    pub query_id: maestria_domain::QueryId,
    pub outcome: SearchOutcome,
    pub resources: ResourceMetrics,
    pub security: SecurityMetrics,
}

/// Regression thresholds for a frozen golden corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoldenGateConfig {
    pub min_recall_at_k: Metric,
    pub min_ndcg_at_k: Metric,
    pub min_mrr: Metric,
    pub min_exact_span_recall: Metric,
    pub max_latency_ms: u64,
    pub max_memory_bytes: u64,
    pub max_disk_bytes: u64,
    pub max_acl_leakage: u32,
    pub max_attack_successes: u32,
}

/// A deterministic gate over a versioned set of golden observations.
pub struct GoldenGate {
    pub config: GoldenGateConfig,
    pub k: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GoldenGateError {
    #[error("golden corpus must contain at least one query")]
    EmptyCorpus,
    #[error("golden gate k must be greater than zero")]
    InvalidK,
    #[error("unsupported golden corpus schema version {0}")]
    UnsupportedSchema(u32),
    #[error("missing observation for query {0}")]
    MissingObservation(u64),
    #[error("duplicate observation for query {0}")]
    DuplicateObservation(u64),
    #[error("duplicate golden query {0}")]
    DuplicateQuery(u64),
    #[error("duplicate judgment for query {query_id}, evidence {evidence_id}")]
    DuplicateJudgment { query_id: u64, evidence_id: u64 },
    #[error("golden query {0} must contain at least one judgment")]
    EmptyJudgments(u64),
    #[error("golden query {0} must contain at least one relevant judgment")]
    NoRelevantJudgments(u64),
    #[error("unexpected observation for query {0}")]
    UnexpectedObservation(u64),
    #[error("query {query_id} does not match its trace")]
    TraceMismatch { query_id: u64 },
    #[error("query {query_id} returned status {found:?}, expected {expected:?}")]
    StatusMismatch {
        query_id: u64,
        expected: maestria_domain::SearchStatus,
        found: maestria_domain::SearchStatus,
    },
    #[error("query {query_id} failed golden gate: {reason}")]
    Regression { query_id: u64, reason: String },
}

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

    fn check_thresholds(
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
        if report.resources.latency_ms > self.config.max_latency_ms {
            return Err(regression(query_id, "latency"));
        }
        if report.resources.memory_bytes > self.config.max_memory_bytes {
            return Err(regression(query_id, "memory"));
        }
        if report.resources.disk_bytes > self.config.max_disk_bytes {
            return Err(regression(query_id, "disk"));
        }
        if report.security.acl_leakage > self.config.max_acl_leakage {
            return Err(regression(query_id, "ACL leakage"));
        }
        if report.security.attack_successes > self.config.max_attack_successes {
            return Err(regression(query_id, "attack success"));
        }
        Ok(())
    }
}

fn validate_inputs<'a>(
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
