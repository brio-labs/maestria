//! Contract benchmark for the learned-sparse four-profile evaluation.
//!
//! The executor is deterministic and backed by the in-memory sparse provider
//! and index fixtures; it is explicitly not promotion evidence. Provider-side
//! telemetry (energy) is marked `Unavailable`, so the gate can never yield a
//! promotion from this report. When `MAESTRIA_BENCHMARK_REPORT_DIR` is set the
//! report is written for the evidence ledger validator.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use maestria_domain::{ContentHash, IndexGenerationId, SearchExecutionBudget};
use maestria_ports::learned_sparse_contract_tests::fixture_sparse_identity;
use maestria_retrieval::{
    CheckStatus, LearnedSparseAcceptedSpan, LearnedSparseBenchmarkCase,
    LearnedSparseBenchmarkComparison, LearnedSparseBenchmarkCorpus,
    LearnedSparseBenchmarkIdentity, LearnedSparseClassDecision, LearnedSparseDataFidelity,
    LearnedSparseEnvironment, LearnedSparseExpectedOutcome, LearnedSparseOperationMeasurement,
    LearnedSparseProviderDisclosure, LearnedSparseQualityMetrics, LearnedSparseQueryClass,
    LearnedSparseResourceMetrics, LearnedSparseRetentionPolicy, LearnedSparseRetrievedCandidate,
    LearnedSparseRetrievedSpan, LearnedSparseRoute, LearnedSparseRouteConfiguration,
    LearnedSparseSafetyMetrics, LearnedSparseTaskCorpus, Measurement,
    run_learned_sparse_benchmark, score_case,
};

const TASK_CORPUS: &str = include_str!("../../../../tests/contracts/learned_sparse_task_corpus_v1.json");
const ROUTE_CONFIGURATIONS: &str =
    include_str!("../../../../tests/contracts/learned_sparse_benchmark_v2.json");

/// Labels bound to the checked-in ledger entry; the real evaluation (Stage C)
/// replaces them with the measured instance fingerprints.
const INDEX_GENERATION_LABEL: &str = "sparse-text-v1-contract-fixture";
const MODEL_FINGERPRINT: &str = "sparse-fixture-model-v1";

fn route_configurations() -> Result<
    BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
    Box<dyn std::error::Error>,
> {
    #[derive(serde::Deserialize)]
    struct ConfigDocument {
        #[serde(default)]
        environment: LearnedSparseEnvironment,
        route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
    }
    let document: ConfigDocument = serde_json::from_str(ROUTE_CONFIGURATIONS)?;
    let _ = document.environment;
    Ok(document.route_configurations)
}

fn benchmark_corpus() -> Result<LearnedSparseBenchmarkCorpus, Box<dyn std::error::Error>> {
    let task: LearnedSparseTaskCorpus = serde_json::from_str(TASK_CORPUS)?;
    let identity = fixture_sparse_identity()?;
    let corpus = task.to_benchmark_corpus(
        environment()?,
        route_configurations()?,
        identity.corpus_snapshot,
        identity.generation_id,
        identity.namespace,
    )?;
    Ok(corpus)
}

fn environment() -> Result<LearnedSparseEnvironment, Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct ConfigDocument {
        environment: LearnedSparseEnvironment,
    }
    let document: ConfigDocument = serde_json::from_str(ROUTE_CONFIGURATIONS)?;
    Ok(document.environment)
}

struct ContractFixtureExecutor {
    corpus_id: String,
    corpus_revision: String,
    judgment_set_id: String,
    evaluation_date: String,
    identity: LearnedSparseBenchmarkIdentity,
    route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
}

impl ContractFixtureExecutor {
    fn new(corpus: &LearnedSparseBenchmarkCorpus) -> Result<Self, Box<dyn std::error::Error>> {
        let identity = LearnedSparseBenchmarkIdentity::from_sparse_identity(
            &fixture_sparse_identity()?,
            "in-memory-contract-fixture-v1",
        )?;
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.corpus_revision.clone(),
            judgment_set_id: corpus.judgment_set_id.clone(),
            evaluation_date: corpus.evaluation_date.clone(),
            identity,
            route_configurations: corpus.route_configurations.clone(),
        })
    }

    fn candidates(
        &self,
        case: &LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Vec<LearnedSparseRetrievedCandidate> {
        let LearnedSparseExpectedOutcome::Evidence { accepted_spans, .. } =
            case.expected.as_ref().unwrap()
        else {
            return Vec::new();
        };
        let spans: Vec<&LearnedSparseAcceptedSpan> = match route {
            // Lexical and the sparse-only ablation both surface only the
            // first accepted span: partial, deterministic coverage.
            LearnedSparseRoute::Lexical | LearnedSparseRoute::SparseOnly => {
                accepted_spans.iter().take(1).collect()
            }
            LearnedSparseRoute::Hybrid | LearnedSparseRoute::SparseFused => {
                accepted_spans.iter().collect()
            }
        };
        spans
            .into_iter()
            .enumerate()
            .map(|(index, span)| LearnedSparseRetrievedCandidate {
                evidence_id: format!("{}-{}", case.case_id, span.source_id),
                lane_rank: index as u32 + 1,
                span: LearnedSparseRetrievedSpan {
                    source_id: span.source_id.clone(),
                    start: span.start,
                    end: span.end,
                },
                citation: Some(LearnedSparseRetrievedSpan {
                    source_id: span.source_id.clone(),
                    start: span.start,
                    end: span.end,
                }),
                grade: Some(2),
            })
            .collect()
    }

    fn resource_latencies(route: LearnedSparseRoute) -> (u64, u64, u64) {
        match route {
            LearnedSparseRoute::Lexical => (30, 40, 50),
            LearnedSparseRoute::Hybrid => (60, 70, 80),
            LearnedSparseRoute::SparseOnly => (70, 80, 90),
            LearnedSparseRoute::SparseFused => (65, 75, 85),
        }
    }

    fn resources(&self, route: LearnedSparseRoute) -> LearnedSparseResourceMetrics {
        let (p50, p95, p99) = Self::resource_latencies(route);
        let operation = LearnedSparseOperationMeasurement {
            elapsed_ms: Measurement::measured(100),
            throughput_items_per_second: Measurement::measured(1_000),
            cost_micros: Measurement::measured(100_000),
            energy_millijoules: Measurement::unavailable(
                "energy requires provider-side RAPL telemetry unavailable in the contract fixture",
            ),
        };
        LearnedSparseResourceMetrics {
            p50_latency_ms: Measurement::measured(p50),
            p95_latency_ms: Measurement::measured(p95),
            p99_latency_ms: Measurement::measured(p99),
            peak_ram_bytes: Measurement::measured(67_108_864),
            index_disk_bytes: Measurement::measured(134_217_728),
            initial_indexing: operation.clone(),
            incremental_update: operation.clone(),
            deletion: operation.clone(),
            rebuild: operation.clone(),
            activation: operation.clone(),
            rollback: operation,
        }
    }

    fn safety(&self) -> LearnedSparseSafetyMetrics {
        LearnedSparseSafetyMetrics {
            provider: Measurement::measured(LearnedSparseProviderDisclosure {
                remote: false,
                retention: LearnedSparseRetentionPolicy::NoRetention,
            }),
            namespace_isolation: Measurement::measured(CheckStatus::Passed),
            acl_leakage: Measurement::measured(0),
            attack_outcome: Measurement::measured(CheckStatus::Passed),
            poisoning_outcome: Measurement::measured(CheckStatus::Passed),
            secret_exposure: Measurement::measured(CheckStatus::NotDetected),
            quarantine_outcome: Measurement::measured(CheckStatus::Passed),
            prompt_injection_outcome: Measurement::measured(CheckStatus::Passed),
            fail_open_count: Measurement::measured(0),
            energy: Measurement::unavailable(
                "energy requires provider-side RAPL telemetry unavailable in the contract fixture",
            ),
        }
    }
}

impl maestria_retrieval::LearnedSparseBenchmarkExecutor for ContractFixtureExecutor {
    fn observe(
        &self,
        case: LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Result<maestria_retrieval::LearnedSparseBenchmarkObservation, maestria_retrieval::LearnedSparseBenchmarkError>
    {
        let expected = case
            .expected
            .clone()
            .ok_or_else(|| {
                maestria_retrieval::LearnedSparseBenchmarkError::InvalidCorpus(format!(
                    "case {} has no expected outcome",
                    case.case_id
                ))
            })?;
        let candidates = self.candidates(&case, route);
        let quality: LearnedSparseQualityMetrics =
            score_case(&case.case_id, &expected, &candidates)?;
        Ok(maestria_retrieval::LearnedSparseBenchmarkObservation {
            schema_version: 2,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            judgment_set_id: self.judgment_set_id.clone(),
            evaluation_date: self.evaluation_date.clone(),
            case_id: case.case_id,
            route,
            identity: self.identity.clone(),
            route_configuration: self
                .route_configurations
                .get(&route)
                .cloned()
                .ok_or_else(|| {
                    maestria_retrieval::LearnedSparseBenchmarkError::InvalidCorpus(format!(
                        "route {route:?} configuration is missing"
                    ))
                })?,
            quality,
            resources: self.resources(route),
            safety: self.safety(),
        })
    }
}

fn decisions_from_comparison(
    comparison: &LearnedSparseBenchmarkComparison,
) -> BTreeMap<LearnedSparseQueryClass, LearnedSparseClassDecision> {
    comparison
        .classes()
        .iter()
        .map(|(class, entry)| {
            let decision = match entry.winning_route {
                Some(LearnedSparseRoute::SparseFused) => {
                    LearnedSparseClassDecision::PromoteSparseFused
                }
                _ if matches!(
                    class,
                    LearnedSparseQueryClass::ExactLiteral
                        | LearnedSparseQueryClass::NoEvidence
                        | LearnedSparseQueryClass::Security
                ) =>
                {
                    LearnedSparseClassDecision::RetainLexical
                }
                _ => LearnedSparseClassDecision::RetainHybrid,
            };
            (*class, decision)
        })
        .collect()
}

#[test]
fn learned_sparse_contract_benchmark_covers_all_cases_and_never_promotes() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = benchmark_corpus()?;
    corpus.validate()?;
    assert_eq!(corpus.cases.len(), 18);
    let classes = corpus
        .cases
        .iter()
        .map(|case| case.class)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(classes, LearnedSparseQueryClass::all().into_iter().collect());
    let executor = ContractFixtureExecutor::new(&corpus)?;
    let observations = run_learned_sparse_benchmark(&corpus, &executor)?;
    assert_eq!(observations.len(), corpus.cases.len() * 4);

    let comparison = LearnedSparseBenchmarkComparison::evaluate(&corpus, &observations)?;
    for (class, entry) in comparison.classes() {
        assert_ne!(
            entry.winning_route,
            Some(LearnedSparseRoute::SparseFused),
            "contract fixture must never win a promotion for {class:?}"
        );
    }
    let promotion = comparison.promotion(
        "learned-sparse-contract-fixture".to_string(),
        corpus.evaluation_date.clone(),
        maestria_retrieval::LearnedSparseRollbackTarget {
            route: LearnedSparseRoute::Hybrid,
            index_generation: IndexGenerationId::new(1),
        },
        ContentHash::new("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())?,
    )?;
    assert!(
        promotion.validate().is_err(),
        "fixture-based promotion records must be invalid"
    );

    if let Ok(report_dir) = std::env::var("MAESTRIA_BENCHMARK_REPORT_DIR") {
        #[derive(serde::Serialize)]
        struct ReportObservation<'a> {
            case_id: &'a str,
            route: LearnedSparseRoute,
            quality: &'a LearnedSparseQualityMetrics,
            resources: &'a LearnedSparseResourceMetrics,
            safety: &'a LearnedSparseSafetyMetrics,
            measurement_status: &'static str,
        }
        #[derive(serde::Serialize)]
        struct Report<'a> {
            measurement_kind: &'static str,
            evaluation_date: &'a str,
            corpus_id: &'a str,
            corpus_revision: &'a str,
            index_generation: &'static str,
            model_fingerprint: &'static str,
            namespace: &'a str,
            route_configuration: &'a LearnedSparseRouteConfiguration,
            observations: Vec<ReportObservation<'a>>,
            decisions: BTreeMap<LearnedSparseQueryClass, LearnedSparseClassDecision>,
        }
        let namespace = {
            let identity = executor.identity.namespace.clone();
            format!(
                "{}:{:?}:{}",
                identity.instance_id(),
                identity.trust_zone(),
                identity.projection()
            )
        };
        fs::create_dir_all(&report_dir)?;
        let report = Report {
            measurement_kind: "learned_sparse_four_profile",
            evaluation_date: &corpus.evaluation_date,
            corpus_id: &corpus.corpus_id,
            corpus_revision: &corpus.corpus_revision,
            index_generation: INDEX_GENERATION_LABEL,
            model_fingerprint: MODEL_FINGERPRINT,
            namespace: &namespace,
            route_configuration: executor
                .route_configurations
                .get(&LearnedSparseRoute::SparseFused)
                .ok_or("sparse-fused route configuration is missing")?,
            observations: observations
                .iter()
                .map(|observation| ReportObservation {
                    case_id: &observation.case_id,
                    route: observation.route,
                    quality: &observation.quality,
                    resources: &observation.resources,
                    safety: &observation.safety,
                    measurement_status: "Measured",
                })
                .collect(),
            decisions: decisions_from_comparison(&comparison),
        };
        fs::write(
            Path::new(&report_dir).join("learned-sparse.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(())
}

#[test]
fn learned_sparse_contract_environment_and_budgets_are_positive() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = benchmark_corpus()?;
    corpus.environment.validate()?;
    for case in &corpus.cases {
        assert!(case.latency_budget_ms > 0);
        assert!(case.memory_budget_bytes > 0);
        assert!(case.disk_budget_bytes > 0);
        assert!(case.ingest_update_budget_ms > 0);
        assert!(case.energy_budget_millijoules > 0);
        assert_ne!(case.fidelity, LearnedSparseDataFidelity::SyntheticContractFixture);
    }
    let budget = SearchExecutionBudget::new(20, 50, 1_000, 0)?;
    let _ = budget;
    Ok(())
}
