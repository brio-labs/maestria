//! Versioned visual benchmark evidence schema (Rule 13: one concept per
//! module). These types are the frozen, immutable representation of visual
//! query classes, page/region judgments, corpus cases, and measured
//! observations; evaluation and promotion decision logic lives in the
//! parent module and in `runner`.

use crate::golden::Metric;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Frozen visual query classes covered by the benchmark gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualQueryClass {
    Text,
    Table,
    Chart,
    Figure,
    Formula,
    ScannedPage,
}

impl VisualQueryClass {
    pub const fn all() -> [Self; 6] {
        [
            Self::Text,
            Self::Table,
            Self::Chart,
            Self::Figure,
            Self::Formula,
            Self::ScannedPage,
        ]
    }

    /// Classify only explicit visual-document query vocabulary.
    pub fn classify(query: &str) -> Option<Self> {
        let normalized = query.to_ascii_lowercase();
        let tokens = normalized
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .collect::<BTreeSet<_>>();
        let visual_document =
            tokens.contains("pdf") || tokens.contains("page") || tokens.contains("document");
        if tokens.contains("scanned") || tokens.contains("scan") || tokens.contains("ocr") {
            Some(Self::ScannedPage)
        } else if tokens.contains("formula")
            || tokens.contains("equation")
            || tokens.contains("mathematical")
        {
            Some(Self::Formula)
        } else if tokens.contains("table") {
            Some(Self::Table)
        } else if tokens.contains("chart") || tokens.contains("graph") {
            Some(Self::Chart)
        } else if tokens.contains("figure") || tokens.contains("diagram") {
            Some(Self::Figure)
        } else if visual_document && (tokens.contains("text") || tokens.contains("paragraph")) {
            Some(Self::Text)
        } else {
            None
        }
    }
}

/// Route compared by the frozen visual benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualRoute {
    TextLayout,
    Visual,
}

/// Page or region shape expected by a visual judgment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualEvidenceKind {
    Page,
    Region,
}

/// Exact immutable page or region evidence location in the frozen corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualEvidenceLocation {
    pub source_path: String,
    pub page: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One frozen page/region judgment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualJudgment {
    pub kind: VisualEvidenceKind,
    pub relevance: u8,
    pub evidence: VisualEvidenceLocation,
}

/// One frozen visual query and its resource budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualBenchmarkCase {
    pub case_id: String,
    pub class: VisualQueryClass,
    pub query: String,
    pub judgments: Vec<VisualJudgment>,
    pub latency_budget_ms: u64,
    pub memory_budget_bytes: u64,
    pub disk_budget_bytes: u64,
    pub energy_budget_millijoules: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Versioned, frozen visual retrieval benchmark corpus.
pub struct VisualBenchmarkCorpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub source_paths: Vec<String>,
    /// ISO‑8601 date of the original corpus freeze.
    #[serde(default)]
    pub evaluation_date: String,
    /// Human‑readable context for this evaluation corpus.
    #[serde(default)]
    pub evaluation_context: String,
    pub cases: Vec<VisualBenchmarkCase>,
}

/// Availability of the provider used for one measured route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VisualProviderStatus {
    Available,
    Degraded { reason: String },
    Unavailable { reason: String },
}

impl VisualProviderStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Measurements for one case and one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualBenchmarkObservation {
    pub corpus_id: String,
    pub corpus_revision: String,
    /// ISO‑8601 timestamp of when the measurement was taken.
    #[serde(default)]
    pub evaluation_date: String,
    /// Fingerprint of the model or provider that produced the measurements.
    #[serde(default)]
    pub model_fingerprint: String,
    /// Serialised provider‑configuration snapshot at measurement time.
    #[serde(default)]
    pub provider_config: serde_json::Value,
    pub case_id: String,
    pub route: VisualRoute,
    pub page_region_recall: Metric,
    pub ndcg_at_10: Metric,
    pub citation_alignment: Metric,
    pub latency_ms: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub energy_millijoules: u64,
    pub privacy_violations: u32,
    pub security_violations: u32,
    pub provider_status: VisualProviderStatus,
}

/// Aggregated metrics for one visual query class and route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualRouteMetrics {
    pub page_region_recall: Metric,
    pub ndcg_at_10: Metric,
    pub citation_alignment: Metric,
    pub p95_latency_ms: u64,
    pub peak_memory_bytes: u64,
    pub peak_disk_bytes: u64,
    pub energy_millijoules: u64,
    pub privacy_violations: u32,
    pub security_violations: u32,
}
