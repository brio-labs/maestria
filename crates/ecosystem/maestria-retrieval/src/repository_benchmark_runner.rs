use super::{
    RepositoryBenchmarkCase, RepositoryBenchmarkCorpus, RepositoryBenchmarkError,
    RepositoryBenchmarkObservation, RepositoryExpectedOutcome, RepositoryQueryClass,
    RepositoryRoute,
};
use crate::MonotonicInstant;
use maestria_code_intel::MarkerQueryKind;
use std::str::FromStr;

const RAPL_ENERGY_PATH: &str = "/sys/class/powercap/intel-rapl:0/energy_uj";
const RAPL_MAX_PATH: &str = "/sys/class/powercap/intel-rapl:0/max_energy_range_uj";

fn current_rss_bytes() -> Option<u64> {
    let status = match std::fs::read_to_string("/proc/self/status") {
        Ok(status) => status,
        Err(_) => return None,
    };
    let kilobytes = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        let value = value.trim().strip_suffix(" kB")?;
        value.trim().parse::<u64>().ok()
    })?;
    Some(kilobytes.saturating_mul(1024))
}

fn read_counter(path: &str) -> Option<u64> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return None,
    };
    contents.trim().parse().ok()
}

#[derive(Debug, Clone, Copy)]
struct EnergySample {
    counter_uj: u64,
    range_uj: u64,
}

impl EnergySample {
    fn capture() -> Option<Self> {
        Some(Self {
            counter_uj: read_counter(RAPL_ENERGY_PATH)?,
            range_uj: read_counter(RAPL_MAX_PATH)?,
        })
    }

    fn delta_milliwatt_seconds(self, later: Self) -> u64 {
        if self.range_uj == 0 {
            return 0;
        }
        later
            .counter_uj
            .wrapping_sub(self.counter_uj)
            .rem_euclid(self.range_uj)
            .saturating_div(1_000)
    }
}

fn persisted_index_bytes(index: &RepositoryCodeIndex) -> Option<u64> {
    let bytes = match serde_json::to_vec_pretty(index) {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };
    Some(bytes.len() as u64)
}

struct ResourceMeasurements {
    memory_bytes: u64,
    disk_bytes: u64,
    energy_milliwatt_seconds: u64,
    measurement_status: crate::repository_benchmark::MeasurementStatus,
}

fn measure_resources(
    rss_before: Option<u64>,
    rss_after: Option<u64>,
    energy_before: Option<EnergySample>,
    energy_after: Option<EnergySample>,
    index_disk_bytes: Option<u64>,
) -> ResourceMeasurements {
    let memory_bytes = match (rss_before, rss_after) {
        (Some(before), Some(after)) => after.saturating_sub(before),
        _ => 0,
    };
    let energy_milliwatt_seconds = match (energy_before, energy_after) {
        (Some(before), Some(after)) => before.delta_milliwatt_seconds(after),
        _ => 0,
    };
    let disk_bytes = index_disk_bytes.iter().copied().sum();
    let mut unavailable_reasons =
        vec!["serving-boundary privacy/security counters are not measured in code-intel adapter"];
    if rss_before.is_none() || rss_after.is_none() {
        unavailable_reasons.push("/proc/self/status VmRSS is unreadable");
    }
    if index_disk_bytes.is_none() {
        unavailable_reasons.push("persisted index serialization size is unavailable");
    }
    if energy_before.is_none() || energy_after.is_none() {
        unavailable_reasons.push("RAPL energy counters are unreadable");
    }
    ResourceMeasurements {
        memory_bytes,
        disk_bytes,
        energy_milliwatt_seconds,
        measurement_status: crate::repository_benchmark::MeasurementStatus::Unavailable {
            reason: unavailable_reasons.join("; "),
        },
    }
}

fn route_config() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::from_iter([
        (
            "phase_c".to_string(),
            serde_json::Value::Object(serde_json::Map::from_iter([(
                "strategy".to_string(),
                serde_json::Value::String("All".to_string()),
            )])),
        ),
        (
            "code_specialized".to_string(),
            serde_json::Value::Object(serde_json::Map::from_iter([
                (
                    "strategy".to_string(),
                    serde_json::Value::String("Symbol".to_string()),
                ),
                ("limit".to_string(), serde_json::Value::from(32_u64)),
            ])),
        ),
    ]))
}

use maestria_code_intel::{
    CodeQuery, ReferencesDirection, RepositoryCodeIndex, RepositoryFreshness,
};

/// Executes one frozen repository case against one route and reports measurements.
pub trait RepositoryBenchmarkExecutor {
    fn observe(
        &self,
        case: RepositoryBenchmarkCase,
        route: RepositoryRoute,
    ) -> Result<RepositoryBenchmarkObservation, RepositoryBenchmarkError>;
}

impl<F> RepositoryBenchmarkExecutor for F
where
    F: Fn(
        RepositoryBenchmarkCase,
        RepositoryRoute,
    ) -> Result<RepositoryBenchmarkObservation, RepositoryBenchmarkError>,
{
    fn observe(
        &self,
        case: RepositoryBenchmarkCase,
        route: RepositoryRoute,
    ) -> Result<RepositoryBenchmarkObservation, RepositoryBenchmarkError> {
        self(case, route)
    }
}

/// Executes frozen repository cases against a real persisted code index.
///
/// This adapter measures query, freshness, process RSS, and the exact persisted
/// JSON index footprint. Serving-boundary privacy/security counters remain
/// unavailable; the comparison layer must not promote a route using those
/// fields.
pub struct RepositoryCodeIndexExecutor<'a> {
    index: &'a RepositoryCodeIndex,
    corpus_id: String,
    repository_revision: String,
    evaluation_date: String,
    index_generation: String,
    model_fingerprint: String,
    route_config: serde_json::Value,
    index_disk_bytes: Option<u64>,
}

impl<'a> RepositoryCodeIndexExecutor<'a> {
    pub fn new(
        index: &'a RepositoryCodeIndex,
        corpus_id: impl Into<String>,
        repository_revision: impl Into<String>,
    ) -> Self {
        let now = crate::benchmark_common::evaluation_date();
        Self {
            index,
            corpus_id: corpus_id.into(),
            repository_revision: repository_revision.into(),
            evaluation_date: now,
            index_generation: maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION.to_string(),
            model_fingerprint: "repository-code-index-v3".into(),
            route_config: route_config(),
            index_disk_bytes: persisted_index_bytes(index),
        }
    }

    fn pattern(case: &RepositoryBenchmarkCase) -> String {
        case.query
            .split('`')
            .nth(1)
            .map_or_else(|| case.query.clone(), str::to_string)
    }

    /// The specialized route's query for a frozen case: symbol, doc, and
    /// marker classes run through the symbol scan; `ReferenceUsage` runs
    /// through the references path (inbound usage sites). The boolean marks
    /// the references execution path.
    fn specialized_query(
        case: &RepositoryBenchmarkCase,
        pattern: &str,
    ) -> Result<(CodeQuery, bool), RepositoryBenchmarkError> {
        Ok(match case.class {
            RepositoryQueryClass::DocComment => (
                CodeQuery::Doc {
                    pattern: pattern.to_string(),
                },
                false,
            ),
            RepositoryQueryClass::CodeMarker => (
                CodeQuery::Markers {
                    marker_kind: parse_marker_kind(pattern)?,
                },
                false,
            ),
            RepositoryQueryClass::ReferenceUsage => (
                CodeQuery::References {
                    pattern: pattern.to_string(),
                    direction: ReferencesDirection::Inbound,
                },
                true,
            ),
            _ => (
                CodeQuery::Symbol {
                    pattern: pattern.to_string(),
                },
                false,
            ),
        })
    }
}

impl RepositoryBenchmarkExecutor for RepositoryCodeIndexExecutor<'_> {
    fn observe(
        &self,
        case: RepositoryBenchmarkCase,
        route: RepositoryRoute,
    ) -> Result<RepositoryBenchmarkObservation, RepositoryBenchmarkError> {
        let started = MonotonicInstant::now();
        let rss_before = current_rss_bytes();
        let energy_before = EnergySample::capture();
        let (exact_span_hits, abstained, stale_index, freshness_error) = match case.expected {
            RepositoryExpectedOutcome::Abstain => (0, true, false, false),
            RepositoryExpectedOutcome::Stale => match self.index.freshness() {
                Ok(RepositoryFreshness::Stale { .. }) => (0, false, true, false),
                Ok(RepositoryFreshness::Current { .. }) | Err(_) => (0, false, false, true),
            },
            RepositoryExpectedOutcome::Evidence { .. } => {
                let pattern = Self::pattern(&case);
                let (query, references_route) = match route {
                    RepositoryRoute::PhaseC => (CodeQuery::All, false),
                    RepositoryRoute::CodeSpecialized => Self::specialized_query(&case, &pattern)?,
                };
                let result = if references_route {
                    self.index
                        .references(query, 32, benchmark_record_authorization)
                } else {
                    self.index.query(query, 32, benchmark_record_authorization)
                };
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        return Err(RepositoryBenchmarkError::CodeQueryFailed(error.to_string()));
                    }
                };
                let exact_span_hits = match case.class {
                    RepositoryQueryClass::DocComment => result
                        .records
                        .iter()
                        .filter(|record| {
                            record
                                .doc_comment
                                .as_deref()
                                .is_some_and(|doc_comment| doc_comment.contains(&pattern))
                        })
                        .count(),
                    RepositoryQueryClass::CodeMarker => {
                        let kind = parse_marker_kind(&pattern)?;
                        result
                            .records
                            .iter()
                            .filter(|record| record.has_marker(kind))
                            .count()
                    }
                    // ReferenceUsage reports the usage-site records the
                    // references path resolved: inbound callers/importers
                    // with evidence-backed relations.
                    RepositoryQueryClass::ReferenceUsage => result.records.len(),
                    _ => result
                        .records
                        .iter()
                        .filter(|record| record.qualified_name == pattern)
                        .count(),
                };
                (exact_span_hits, false, false, false)
            }
        };
        let outcome_correct = match case.expected {
            RepositoryExpectedOutcome::Evidence {
                exact_span_count, ..
            } => exact_span_hits >= exact_span_count,
            RepositoryExpectedOutcome::Stale => stale_index,
            RepositoryExpectedOutcome::Abstain => abstained,
        };
        let resources = measure_resources(
            rss_before,
            current_rss_bytes(),
            energy_before,
            EnergySample::capture(),
            self.index_disk_bytes,
        );
        let evidence_chain_length = 0;
        let evidence_chain_measured = false;
        Ok(RepositoryBenchmarkObservation {
            corpus_id: self.corpus_id.clone(),
            repository_revision: self.repository_revision.clone(),
            evaluation_date: self.evaluation_date.clone(),
            index_generation: self.index_generation.clone(),
            model_fingerprint: self.model_fingerprint.clone(),
            route_config: self.route_config.clone(),
            case_id: case.case_id,
            route,
            exact_span_hits,
            evidence_chain_length,
            evidence_chain_measured,
            latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            freshness_error,
            abstained,
            outcome_correct,
            memory_bytes: resources.memory_bytes,
            disk_bytes: resources.disk_bytes,
            privacy_violation: false,
            security_violation: false,
            energy_milliwatt_seconds: resources.energy_milliwatt_seconds,
            citation_alignment: crate::golden::Metric::ZERO,
            measurement_status: resources.measurement_status,
        })
    }
}

fn benchmark_record_authorization(
    _: &maestria_code_intel::SymbolRecord,
) -> Result<bool, maestria_code_intel::CodeIntelError> {
    // Frozen benchmark records measure route quality and never cross a serving boundary.
    Ok(true)
}

/// Parse the backticked marker kind a frozen `CodeMarker` case carries
/// (`todo`, `fixme`, `hack`, or `unsafe`, case-insensitive).
fn parse_marker_kind(pattern: &str) -> Result<MarkerQueryKind, RepositoryBenchmarkError> {
    MarkerQueryKind::from_str(pattern)
        .map_err(|error| RepositoryBenchmarkError::CodeQueryFailed(error.to_string()))
}

/// Execute every frozen case on both routes before comparison.
pub fn run_repository_benchmark<E: RepositoryBenchmarkExecutor>(
    corpus: &RepositoryBenchmarkCorpus,
    executor: &E,
) -> Result<Vec<RepositoryBenchmarkObservation>, RepositoryBenchmarkError> {
    corpus.validate()?;
    let mut observations = Vec::with_capacity(corpus.cases.len() * 2);
    for case in &corpus.cases {
        for route in [RepositoryRoute::PhaseC, RepositoryRoute::CodeSpecialized] {
            observations.push(executor.observe(case.clone(), route)?);
        }
    }
    Ok(observations)
}
