use maestria_retrieval::repository_benchmark::{
    RepositoryBenchmarkCase, RepositoryBenchmarkComparison, RepositoryBenchmarkCorpus,
    RepositoryBenchmarkError, RepositoryBenchmarkObservation, RepositoryCodeIndexExecutor,
    RepositoryExecutionPolicy, RepositoryQueryClass, RepositoryRoute, run_repository_benchmark,
};
use std::collections::BTreeSet;
use std::fs;

fn rust_repository_benchmark_fixture() -> Result<RepositoryBenchmarkCorpus, RepositoryBenchmarkError>
{
    let fixture = include_str!("fixtures/rust-repository-benchmark-v1.json");
    RepositoryBenchmarkCorpus::from_json(fixture)
}

fn expected_class_set() -> BTreeSet<RepositoryQueryClass> {
    RepositoryQueryClass::all().into_iter().collect()
}

fn case_by_id<'a>(
    corpus: &'a RepositoryBenchmarkCorpus,
    case_id: &str,
) -> Option<&'a RepositoryBenchmarkCase> {
    corpus.cases.iter().find(|case| case.case_id == case_id)
}

fn repository_benchmark_observations(
    corpus: &RepositoryBenchmarkCorpus,
) -> Result<Vec<RepositoryBenchmarkObservation>, RepositoryBenchmarkError> {
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .map_err(|e| RepositoryBenchmarkError::InvalidCorpus(e.to_string()))?;
    let index = maestria_code_intel::RepositoryCodeIndex::build_with_exclusions(
        repository_root,
        maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION,
        &[],
    )
    .map_err(|e| RepositoryBenchmarkError::InvalidCorpus(e.to_string()))?;
    let executor = RepositoryCodeIndexExecutor::new(
        &index,
        corpus.corpus_id.clone(),
        index.summary.commit_sha.clone(),
    );
    run_repository_benchmark(corpus, &executor)
}

#[test]
fn repository_benchmark_fixture_parses_and_covers_all_required_query_classes()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;

    assert_eq!(corpus.cases.len(), 7);
    corpus.validate()?;
    let classes: BTreeSet<_> = corpus.cases.iter().map(|case| case.class).collect();
    assert_eq!(classes.len(), 7);
    assert_eq!(classes, expected_class_set());

    let ids: BTreeSet<_> = corpus
        .cases
        .iter()
        .map(|case| case.case_id.as_str())
        .collect();
    assert_eq!(ids.len(), 7);

    for case in &corpus.cases {
        assert_eq!(
            RepositoryQueryClass::classify(&case.query),
            Some(case.class)
        );
        assert!(!case.query.trim().is_empty());
        assert!(case.latency_budget_ms > 0);
    }

    Ok(())
}

#[test]
fn repository_benchmark_policy_defaults_to_shadowed_phasec_routing()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let policy = RepositoryExecutionPolicy::default();

    for case in &corpus.cases {
        assert!(!policy.allows_specialized(&case.query));
        assert_eq!(policy.route_for(&case.query), RepositoryRoute::PhaseC);
    }

    assert_eq!(
        policy.route_for("unrelated repository question"),
        RepositoryRoute::PhaseC
    );
    assert!(!policy.allows_specialized("unrelated repository question"));

    Ok(())
}

#[test]
fn repository_benchmark_comparison_evaluates_real_observations()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let observations = repository_benchmark_observations(&corpus)?;
    let comparison = RepositoryBenchmarkComparison::evaluate(&corpus, &observations)?;
    assert_eq!(
        comparison.classes().len(),
        RepositoryQueryClass::all().len()
    );
    let promotion = comparison.promotion("rust-repository-benchmark-v1".to_owned())?;
    assert!(!promotion.evaluation_id().is_empty());
    assert!(promotion.winning_classes().is_subset(&expected_class_set()));
    assert_eq!(promotion.corpus_id(), &corpus.corpus_id);

    let policy = RepositoryExecutionPolicy::Active(promotion.clone());
    for case in &corpus.cases {
        let should_use_specialized = promotion.winning_classes().contains(&case.class);
        assert_eq!(
            policy.allows_specialized(&case.query),
            should_use_specialized
        );
    }
    Ok(())
}

#[test]
fn repository_benchmark_latency_regression_blocks_promotion()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let mut observations = repository_benchmark_observations(&corpus)?;
    for observation in observations.iter_mut().filter(|observation| {
        observation.route == RepositoryRoute::CodeSpecialized
            && case_by_id(&corpus, &observation.case_id)
                .is_some_and(|case| case.class == RepositoryQueryClass::ExactSymbol)
    }) {
        observation.latency_ms = 200;
    }

    let comparison = RepositoryBenchmarkComparison::evaluate(&corpus, &observations)?;
    let exact = comparison
        .classes()
        .get(&RepositoryQueryClass::ExactSymbol)
        .ok_or("missing exact-symbol comparison")?;
    assert!(!exact.specialized_wins);
    assert!(exact.code_specialized.p95_latency_ms > exact.phase_c.p95_latency_ms);
    Ok(())
}

#[test]
fn repository_benchmark_freshness_regression_blocks_promotion_for_stale_worktree()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let mut observations = repository_benchmark_observations(&corpus)?;

    for observation in observations.iter_mut().filter(|observation| {
        observation.route == RepositoryRoute::CodeSpecialized
            && case_by_id(&corpus, &observation.case_id).is_some_and(|case| {
                matches!(
                    case.class,
                    RepositoryQueryClass::ExactSymbol | RepositoryQueryClass::StaleWorktree
                )
            })
    }) {
        observation.freshness_error = true;
    }

    let comparison = RepositoryBenchmarkComparison::evaluate(&corpus, &observations)?;
    let promotion = comparison.promotion("rust-repository-benchmark-v1-regressed".to_owned())?;
    let exact = comparison
        .classes()
        .get(&RepositoryQueryClass::ExactSymbol)
        .ok_or("missing exact-symbol comparison")?;
    let stale_worktree = comparison
        .classes()
        .get(&RepositoryQueryClass::StaleWorktree)
        .ok_or("missing stale-worktree comparison")?;

    assert!(promotion.winning_classes().is_empty());
    assert!(!exact.specialized_wins);
    assert_eq!(exact.code_specialized.freshness_errors, 1);
    assert!(!stale_worktree.specialized_wins);
    assert_eq!(stale_worktree.code_specialized.freshness_errors, 1);

    Ok(())
}

#[test]
fn repository_benchmark_abstention_safety_blocks_unsafe_specialized_route()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let observations = repository_benchmark_observations(&corpus)?;
    let comparison = RepositoryBenchmarkComparison::evaluate(&corpus, &observations)?;
    let abstention = comparison
        .classes()
        .get(&RepositoryQueryClass::CorrectAbstention)
        .ok_or("missing abstention comparison")?;

    // The abstention class must never be promoted: its specialized route
    // does not demonstrate sufficient quality gain over the Phase‑C baseline,
    // and the safety gate correctly blocks it.
    assert!(!abstention.specialized_wins);

    // Policy must route abstention cases through Phase‑C regardless of
    // whether other classes are promoted.
    let policy = RepositoryExecutionPolicy::Active(
        comparison.promotion("rust-repository-benchmark-v1".to_owned())?,
    );
    let abstention_case = corpus
        .cases
        .iter()
        .find(|case| case.class == RepositoryQueryClass::CorrectAbstention)
        .ok_or("missing abstention fixture case")?;
    assert!(!policy.allows_specialized(&abstention_case.query));
    assert_eq!(
        policy.route_for(&abstention_case.query),
        RepositoryRoute::PhaseC
    );

    Ok(())
}

#[test]
fn repository_benchmark_phase_c_baseline_executes_against_real_index()
-> Result<(), Box<dyn std::error::Error>> {
    // Verify that the Phase‑C baseline route produces deterministic
    // observations against the real code index.
    let corpus = rust_repository_benchmark_fixture()?;
    let observations = repository_benchmark_observations(&corpus)?;
    let phase_c_obs: Vec<_> = observations
        .iter()
        .filter(|o| o.route == RepositoryRoute::PhaseC)
        .collect();
    assert_eq!(phase_c_obs.len(), corpus.cases.len());
    for obs in &phase_c_obs {
        assert!(!obs.case_id.is_empty());
        assert!(!obs.evaluation_date.is_empty());
        assert!(!obs.index_generation.is_empty());
        assert!(!obs.model_fingerprint.is_empty());
        // Every observation must have a measurement status (unavailable is
        // explicit, not an omission).
        assert!(matches!(
            obs.measurement_status,
            maestria_retrieval::MeasurementStatus::Measured
                | maestria_retrieval::MeasurementStatus::Unavailable { .. }
        ));
    }
    Ok(())
}

#[test]
fn real_repository_executor_runs_frozen_cases_against_a_code_index()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let index = maestria_code_intel::RepositoryCodeIndex::build_with_exclusions(
        repository_root,
        maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION,
        &[],
    )?;
    let executor = RepositoryCodeIndexExecutor::new(
        &index,
        corpus.corpus_id.clone(),
        index.summary.commit_sha.clone(),
    );
    let observations = run_repository_benchmark(&corpus, &executor)?;
    assert_eq!(observations.len(), corpus.cases.len() * 2);
    assert!(observations.iter().all(|observation| {
        observation.corpus_id == corpus.corpus_id && !observation.repository_revision.is_empty()
    }));
    assert!(
        observations
            .iter()
            .any(|observation| { observation.route == RepositoryRoute::CodeSpecialized })
    );
    // Verify complete metadata for every observation.
    for obs in &observations {
        assert!(
            !obs.evaluation_date.is_empty(),
            "evaluation_date must be set"
        );
        assert!(
            !obs.index_generation.is_empty(),
            "index_generation must be set"
        );
        assert!(
            !obs.model_fingerprint.is_empty(),
            "model_fingerprint must be set"
        );
        assert_eq!(
            obs.measurement_status,
            maestria_retrieval::MeasurementStatus::Unavailable {
                reason: "platform counters not available in code-intel adapter".into(),
            }
        );
        assert_eq!(obs.disk_bytes, 0, "disk_bytes default is zero (unmeasured)");
        assert_eq!(
            obs.citation_alignment,
            maestria_retrieval::golden::Metric::ZERO
        );
    }

    if let Ok(report_dir) = std::env::var("MAESTRIA_BENCHMARK_REPORT_DIR") {
        #[derive(serde::Serialize)]
        struct Report<'a> {
            measurement_kind: &'static str,
            corpus_id: &'a str,
            repository_revision: &'a str,
            evaluation_date: &'a str,
            index_generation: &'a str,
            model_fingerprint: &'a str,
            route_config: &'a serde_json::Value,
            measurement_status: &'a maestria_retrieval::MeasurementStatus,
            observations: &'a [RepositoryBenchmarkObservation],
        }
        fs::create_dir_all(&report_dir)?;
        let first = &observations[0];
        let report = Report {
            measurement_kind: "real_repository_code_index",
            corpus_id: &corpus.corpus_id,
            repository_revision: &index.summary.commit_sha,
            evaluation_date: &first.evaluation_date,
            index_generation: &first.index_generation,
            model_fingerprint: &first.model_fingerprint,
            route_config: &first.route_config,
            measurement_status: &first.measurement_status,
            observations: &observations,
        };
        fs::write(
            std::path::Path::new(&report_dir).join("repository-real.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(())
}
