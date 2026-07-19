use maestria_retrieval::repository_benchmark::{
    RepositoryBenchmarkCase, RepositoryBenchmarkComparison, RepositoryBenchmarkCorpus,
    RepositoryBenchmarkError, RepositoryBenchmarkObservation, RepositoryExecutionPolicy,
    RepositoryQueryClass, RepositoryRoute, run_repository_benchmark,
};
use std::collections::BTreeSet;

fn rust_repository_benchmark_fixture() -> Result<RepositoryBenchmarkCorpus, RepositoryBenchmarkError>
{
    let fixture = include_str!("fixtures/rust-repository-benchmark-v1.json");
    RepositoryBenchmarkCorpus::from_json(fixture)
}

fn expected_promotion_classes() -> BTreeSet<RepositoryQueryClass> {
    [RepositoryQueryClass::ExactSymbol].into_iter().collect()
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

fn profile(
    case: &RepositoryBenchmarkCase,
    route: RepositoryRoute,
) -> (usize, usize, u64, bool, bool, bool) {
    match (case.class, route) {
        (RepositoryQueryClass::ExactSymbol, RepositoryRoute::PhaseC) => {
            (0, 0, 95, false, false, false)
        }
        (RepositoryQueryClass::ExactSymbol, RepositoryRoute::CodeSpecialized) => {
            (2, 3, 80, true, false, false)
        }
        (RepositoryQueryClass::DefinitionReference, RepositoryRoute::PhaseC) => {
            (1, 0, 95, false, false, false)
        }
        (RepositoryQueryClass::DefinitionReference, RepositoryRoute::CodeSpecialized) => {
            (1, 0, 70, false, false, false)
        }
        (RepositoryQueryClass::IssueToFile, RepositoryRoute::PhaseC) => {
            (0, 0, 95, false, false, false)
        }
        (RepositoryQueryClass::IssueToFile, RepositoryRoute::CodeSpecialized) => {
            (0, 0, 70, false, false, false)
        }
        (RepositoryQueryClass::MultiHopDependency, RepositoryRoute::PhaseC) => {
            (1, 1, 130, false, false, false)
        }
        (RepositoryQueryClass::MultiHopDependency, RepositoryRoute::CodeSpecialized) => {
            (1, 1, 90, false, false, false)
        }
        (RepositoryQueryClass::TestAssociation, RepositoryRoute::PhaseC) => {
            (1, 1, 110, true, false, false)
        }
        (RepositoryQueryClass::TestAssociation, RepositoryRoute::CodeSpecialized) => {
            (1, 1, 90, true, false, false)
        }
        (RepositoryQueryClass::StaleWorktree, RepositoryRoute::PhaseC) => {
            (0, 0, 70, false, false, false)
        }
        (RepositoryQueryClass::StaleWorktree, RepositoryRoute::CodeSpecialized) => {
            (0, 0, 40, false, false, false)
        }
        (RepositoryQueryClass::CorrectAbstention, RepositoryRoute::PhaseC) => {
            (0, 0, 90, false, true, false)
        }
        (RepositoryQueryClass::CorrectAbstention, RepositoryRoute::CodeSpecialized) => {
            (0, 0, 70, true, false, false)
        }
    }
}

fn repository_benchmark_observations(
    corpus: &RepositoryBenchmarkCorpus,
) -> Result<Vec<RepositoryBenchmarkObservation>, RepositoryBenchmarkError> {
    let corpus_id = corpus.corpus_id.clone();
    let repository_revision = corpus.repository_revision.clone();
    run_repository_benchmark(corpus, &move |case, route| {
        let (
            exact_span_hits,
            evidence_chain_length,
            latency_ms,
            outcome_correct,
            abstained,
            freshness_error,
        ) = profile(&case, route);
        let memory_bytes = match route {
            RepositoryRoute::PhaseC => 1_200,
            RepositoryRoute::CodeSpecialized => 900,
        };
        let energy_milliwatt_seconds = match route {
            RepositoryRoute::PhaseC => 120,
            RepositoryRoute::CodeSpecialized => 90,
        };
        Ok(RepositoryBenchmarkObservation {
            corpus_id: corpus_id.clone(),
            repository_revision: repository_revision.clone(),
            case_id: case.case_id.clone(),
            route,
            exact_span_hits,
            evidence_chain_length,
            latency_ms,
            freshness_error,
            abstained,
            outcome_correct,
            memory_bytes,
            privacy_violation: false,
            security_violation: false,
            energy_milliwatt_seconds,
        })
    })
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
fn repository_benchmark_class_comparison_promotes_only_winning_classes()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = rust_repository_benchmark_fixture()?;
    let observations = repository_benchmark_observations(&corpus)?;
    let comparison = RepositoryBenchmarkComparison::evaluate(&corpus, &observations)?;
    let promotion = comparison.promotion("rust-repository-benchmark-v1".to_owned())?;

    assert_eq!(promotion.winning_classes(), &expected_promotion_classes());

    let exact = comparison
        .classes()
        .get(&RepositoryQueryClass::ExactSymbol)
        .ok_or("missing exact symbol comparison")?;
    assert!(exact.specialized_wins);
    assert!(
        exact.code_specialized.exact_span_recall.value() > exact.phase_c.exact_span_recall.value()
    );
    assert!(
        exact.code_specialized.evidence_chain_accuracy.value()
            > exact.phase_c.evidence_chain_accuracy.value()
    );
    assert!(exact.code_specialized.peak_memory_bytes < exact.phase_c.peak_memory_bytes);
    assert!(
        exact.code_specialized.energy_milliwatt_seconds < exact.phase_c.energy_milliwatt_seconds
    );

    let abstention = comparison
        .classes()
        .get(&RepositoryQueryClass::CorrectAbstention)
        .ok_or("missing abstention comparison")?;
    assert!(!abstention.specialized_wins);
    assert!(
        abstention.code_specialized.outcome_accuracy.value()
            > abstention.phase_c.outcome_accuracy.value()
    );
    assert!(
        abstention.code_specialized.abstention_accuracy.value()
            < abstention.phase_c.abstention_accuracy.value()
    );

    let policy = RepositoryExecutionPolicy::Active(promotion.clone());
    for case in &corpus.cases {
        assert!(case_by_id(&corpus, &case.case_id).is_some());
        let should_use_specialized = promotion.winning_classes().contains(&case.class);
        assert_eq!(
            policy.allows_specialized(&case.query),
            should_use_specialized
        );
        assert_eq!(
            policy.route_for(&case.query),
            if should_use_specialized {
                RepositoryRoute::CodeSpecialized
            } else {
                RepositoryRoute::PhaseC
            }
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

    assert!(!abstention.specialized_wins);
    assert!(
        abstention.code_specialized.outcome_accuracy.value()
            > abstention.phase_c.outcome_accuracy.value()
    );
    assert!(
        abstention.code_specialized.abstention_accuracy.value()
            < abstention.phase_c.abstention_accuracy.value()
    );

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
