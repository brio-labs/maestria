use maestria_domain::{EvidenceId, EvidenceSpan, SearchOutcome, SearchTrace};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
#[path = "golden_metrics.rs"]
mod golden_metrics;
pub(crate) use golden_metrics::calculate_report;
#[path = "golden_comparison.rs"]
mod golden_comparison;
pub use golden_comparison::{GoldenComparison, PromotionRecord};
#[path = "golden_comparison_regressions.rs"]
mod golden_comparison_regressions;
#[path = "golden_gate_eval.rs"]
mod golden_gate_eval;
pub(crate) use golden_gate_eval::validate_inputs;

/// Fixed-point metric in the inclusive range 0..=10_000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Metric(u32);

impl Metric {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(10_000);
    /// Five percentage points on the fixed-point quality scale.
    pub const MATERIAL_QUALITY_DELTA: Self = Self(500);

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
    /// Optional full trace snapshot for route and provenance regression checks.
    #[serde(default)]
    pub expected_trace: Option<SearchTrace>,
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
    #[serde(default)]
    pub ingest_update_ms: Option<u64>,
    #[serde(default)]
    pub energy_millijoules: Option<u64>,
    #[serde(default)]
    pub telemetry_complete: bool,
}

impl ResourceMetrics {
    pub fn measured() -> Self {
        Self {
            telemetry_complete: true,
            ..Self::default()
        }
    }
}

/// Security measurements supplied by retrieval validators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityMetrics {
    pub acl_leakage: u32,
    pub attack_successes: u32,
    #[serde(default)]
    pub privacy_violations: u32,
    #[serde(default)]
    pub telemetry_complete: bool,
}

impl SecurityMetrics {
    pub fn measured() -> Self {
        Self {
            telemetry_complete: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenObservation {
    pub query_id: maestria_domain::QueryId,
    pub profile: GoldenProfile,
    pub outcome: SearchOutcome,
    pub resources: ResourceMetrics,
    pub security: SecurityMetrics,
}

/// A serialized corpus and its observations for one deterministic evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenFixture {
    pub corpus: GoldenCorpus,
    pub observations: Vec<GoldenObservation>,
}

impl GoldenFixture {
    /// Evaluate the persisted fixture with the configured deterministic gate.
    pub fn evaluate(
        &self,
        gate: &GoldenGate,
    ) -> Result<Vec<GoldenEvaluationReport>, GoldenGateError> {
        gate.evaluate_fixture(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoldenProfile {
    #[serde(rename = "v0.4")]
    V0_4,
    #[serde(rename = "v0.5")]
    V0_5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendTier {
    #[serde(rename = "S")]
    Small,
    #[serde(rename = "M")]
    Medium,
    #[serde(rename = "L")]
    Large,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedQualityMetrics {
    pub mean_recall_at_k: Metric,
    pub mean_ndcg_at_k: Metric,
    pub mean_mrr: Metric,
    pub mean_exact_span_recall: Metric,
}

impl Default for AggregatedQualityMetrics {
    fn default() -> Self {
        Self {
            mean_recall_at_k: Metric::ZERO,
            mean_ndcg_at_k: Metric::ZERO,
            mean_mrr: Metric::ZERO,
            mean_exact_span_recall: Metric::ZERO,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedResourceMetrics {
    pub p50_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub p99_latency_ms: u64,
    pub peak_memory_bytes: u64,
    pub peak_disk_bytes: u64,
    pub total_ingest_update_ms: Option<u64>,
    pub total_energy_millijoules: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedSecurityMetrics {
    pub total_acl_leakage: u32,
    pub total_attack_successes: u32,
    pub total_privacy_violations: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenComparisonReport {
    pub backend_tier: BackendTier,
    pub workload: String,
    pub corpus_snapshot: maestria_domain::CorpusSnapshotId,
    pub index_generation: maestria_domain::IndexGenerationId,
    pub fingerprint: maestria_domain::RetrievalModelFingerprint,
    pub baseline_profile: GoldenProfile,
    pub candidate_profile: GoldenProfile,
    pub baseline_quality: AggregatedQualityMetrics,
    pub candidate_quality: AggregatedQualityMetrics,
    pub baseline_resources: AggregatedResourceMetrics,
    pub candidate_resources: AggregatedResourceMetrics,
    pub baseline_security: AggregatedSecurityMetrics,
    pub candidate_security: AggregatedSecurityMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PromotionDecision {
    Promote {
        evaluation_id: String,
        evaluation_date: String,
        reason: String,
    },
    RetainBaseline {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenComparisonResult {
    pub report: GoldenComparisonReport,
    pub decision: PromotionDecision,
}

impl GoldenComparisonResult {
    pub fn is_promoted(&self) -> bool {
        matches!(self.decision, PromotionDecision::Promote { .. })
    }
}

/// Regression thresholds for a frozen golden corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenGateConfig {
    pub profile: GoldenProfile,
    pub min_recall_at_k: Metric,
    pub min_ndcg_at_k: Metric,
    pub min_mrr: Metric,
    pub min_exact_span_recall: Metric,
    pub min_material_quality_delta: Metric,
    pub max_latency_ms: u64,
    pub max_memory_bytes: u64,
    pub max_disk_bytes: u64,
    #[serde(default)]
    pub max_ingest_update_ms: Option<u64>,
    #[serde(default)]
    pub max_energy_millijoules: Option<u64>,
    pub max_acl_leakage: u32,
    pub max_attack_successes: u32,
    #[serde(default)]
    pub max_privacy_violations: u32,
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
    #[error("query {query_id} uses profile {found:?}, expected {expected:?}")]
    ProfileMismatch {
        query_id: u64,
        expected: GoldenProfile,
        found: GoldenProfile,
    },
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
    #[error("baseline and candidate profiles must be v0.4 and v0.5")]
    InvalidProfilePair,
    #[error("golden comparison workload must be non-empty")]
    InvalidWorkload,
    #[error("promotion requires a non-empty evaluation id and date")]
    MissingPromotionRecord,
}
