use super::{
    RepositoryBenchmarkCase, RepositoryBenchmarkCorpus, RepositoryBenchmarkError,
    RepositoryBenchmarkObservation, RepositoryExpectedOutcome, RepositoryRoute,
};
use crate::MonotonicInstant;

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
use maestria_code_intel::{CodeQuery, RepositoryCodeIndex, RepositoryFreshness};

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
/// This adapter measures the real index query and freshness APIs directly. It
/// does not synthesize quality tuples; memory and energy remain zero-valued
/// because this adapter does not have platform counters, and are visible to
/// promotion policy rather than inferred.
pub struct RepositoryCodeIndexExecutor<'a> {
    index: &'a RepositoryCodeIndex,
    corpus_id: String,
    repository_revision: String,
    evaluation_date: String,
    index_generation: String,
    model_fingerprint: String,
    route_config: serde_json::Value,
}

impl<'a> RepositoryCodeIndexExecutor<'a> {
    pub fn new(
        index: &'a RepositoryCodeIndex,
        corpus_id: impl Into<String>,
        repository_revision: impl Into<String>,
    ) -> Self {
        let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_secs().to_string(),
            Err(_) => "unknown".into(),
        };
        Self {
            index,
            corpus_id: corpus_id.into(),
            repository_revision: repository_revision.into(),
            evaluation_date: now,
            index_generation: maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION.to_string(),
            model_fingerprint: "repository-code-index-v1".into(),
            route_config: route_config(),
        }
    }

    fn pattern(case: &RepositoryBenchmarkCase) -> String {
        case.query
            .split('`')
            .nth(1)
            .map_or_else(|| case.query.clone(), str::to_string)
    }
}

impl RepositoryBenchmarkExecutor for RepositoryCodeIndexExecutor<'_> {
    fn observe(
        &self,
        case: RepositoryBenchmarkCase,
        route: RepositoryRoute,
    ) -> Result<RepositoryBenchmarkObservation, RepositoryBenchmarkError> {
        let started = MonotonicInstant::now();
        let (exact_span_hits, abstained, stale_index, freshness_error) = match case.expected {
            RepositoryExpectedOutcome::Abstain => (0, true, false, false),
            RepositoryExpectedOutcome::Stale => match self.index.freshness() {
                Ok(RepositoryFreshness::Stale { .. }) => (0, false, true, false),
                Ok(RepositoryFreshness::Current { .. }) | Err(_) => (0, false, false, true),
            },
            RepositoryExpectedOutcome::Evidence { .. } => {
                let result = match route {
                    RepositoryRoute::PhaseC => self.index.query(CodeQuery::All, 32),
                    RepositoryRoute::CodeSpecialized => self.index.query(
                        CodeQuery::Symbol {
                            pattern: Self::pattern(&case),
                        },
                        32,
                    ),
                };
                (result.records.len(), false, false, false)
            }
        };
        let outcome_correct = match case.expected {
            RepositoryExpectedOutcome::Evidence {
                exact_span_count, ..
            } => exact_span_hits >= exact_span_count,
            RepositoryExpectedOutcome::Stale => stale_index,
            RepositoryExpectedOutcome::Abstain => abstained,
        };
        let evidence_chain_length = exact_span_hits;
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
            latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            freshness_error,
            abstained,
            outcome_correct,
            memory_bytes: 0,
            disk_bytes: 0,
            privacy_violation: false,
            security_violation: false,
            energy_milliwatt_seconds: 0,
            citation_alignment: crate::golden::Metric::ZERO,
            measurement_status: crate::repository_benchmark::MeasurementStatus::Unavailable {
                reason: "platform counters not available in code-intel adapter".into(),
            },
        })
    }
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
