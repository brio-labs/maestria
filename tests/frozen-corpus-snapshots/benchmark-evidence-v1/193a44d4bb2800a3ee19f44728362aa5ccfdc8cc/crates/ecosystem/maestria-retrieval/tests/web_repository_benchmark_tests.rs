use maestria_retrieval::repository_benchmark::{
    RepositoryBenchmarkCorpus, RepositoryBenchmarkError, RepositoryCodeIndexExecutor,
    RepositoryExpectedOutcome, RepositoryQueryClass, RepositoryRoute, run_repository_benchmark,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const FIXTURE_DIR: &str = "tests/fixtures/web-repository-v1";

fn web_repository_benchmark_fixture() -> Result<RepositoryBenchmarkCorpus, RepositoryBenchmarkError>
{
    let fixture = include_str!("fixtures/web-repository-benchmark-v1.json");
    RepositoryBenchmarkCorpus::from_json(fixture)
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR)
}

/// Copy the frozen fixture into a temp dir, initialize a git repository at
/// the copied state, and return the temp dir plus the fixture commit.
fn fixture_repository() -> Result<(tempfile::TempDir, String), Box<dyn std::error::Error>> {
    let tmp = tempdir()?;
    maestria_test_support::copy_tree(&fixture_root(), tmp.path())?;
    maestria_test_support::run_git(
        tmp.path(),
        &["init", "--initial-branch", "main"],
        "git init",
    )?;
    maestria_test_support::run_git(
        tmp.path(),
        &["config", "user.email", "ci@example.com"],
        "git config user.email",
    )?;
    maestria_test_support::run_git(
        tmp.path(),
        &["config", "user.name", "CI"],
        "git config user.name",
    )?;
    maestria_test_support::run_git(tmp.path(), &["add", "."], "git add")?;
    maestria_test_support::run_git(tmp.path(), &["commit", "-m", "fixture init"], "git commit")?;
    let output = Command::new("git")
        .current_dir(tmp.path())
        .args(["rev-parse", "HEAD"])
        .output()?;
    let sha = String::from_utf8(output.stdout)?.trim().to_string();
    Ok((tmp, sha))
}

#[test]
fn web_repository_benchmark_fixture_parses_and_validates() -> Result<(), Box<dyn std::error::Error>>
{
    let corpus = web_repository_benchmark_fixture()?;

    // The corpus validator enforces one case per required query class; do
    // not assert against `RepositoryQueryClass::all()` here (a parallel
    // issue adds a tenth class; the integrator reconciles).
    corpus.validate()?;
    let classes: BTreeSet<_> = corpus.cases.iter().map(|case| case.class).collect();
    assert_eq!(classes.len(), corpus.cases.len(), "classes must be unique");

    for case in &corpus.cases {
        assert_eq!(
            RepositoryQueryClass::classify(&case.query),
            Some(case.class),
            "case {} must classify as {}",
            case.case_id,
            case.class as usize
        );
        assert!(!case.query.trim().is_empty());
        assert!(case.latency_budget_ms > 0);
    }
    Ok(())
}

#[test]
fn web_repository_corpus_executes_against_the_frozen_fixture_index()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = web_repository_benchmark_fixture()?;
    let (tmp, revision) = fixture_repository()?;

    // Build the reference index from the fixture copy, then run every frozen
    // case on both routes against the real index.
    let index = maestria_code_intel::RepositoryCodeIndex::build_with_exclusions(
        tmp.path(),
        maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION,
        &[],
    )
    .map_err(|error| RepositoryBenchmarkError::InvalidCorpus(error.to_string()))?;
    assert!(
        index.summary.symbol_count > 0,
        "fixture must index real web symbols"
    );
    assert_eq!(index.summary.package_count, 1);
    assert_eq!(index.summary.packages, vec!["ui-kit".to_string()]);
    let executor =
        RepositoryCodeIndexExecutor::new(&index, corpus.corpus_id.clone(), revision.clone());
    let observations = run_repository_benchmark(&corpus, &executor)?;
    assert_eq!(observations.len(), corpus.cases.len() * 2);

    // Every evidence case must hit its verified span count on both routes,
    // and the observations must carry the fixture identity.
    for case in &corpus.cases {
        let case_observations: Vec<_> = observations
            .iter()
            .filter(|observation| observation.case_id == case.case_id)
            .collect();
        assert_eq!(case_observations.len(), 2);
        for observation in case_observations {
            assert_eq!(observation.corpus_id, corpus.corpus_id);
            assert_eq!(observation.repository_revision, revision);
            assert_eq!(
                observation.index_generation,
                maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION
            );
            match &case.expected {
                RepositoryExpectedOutcome::Evidence { .. } => {
                    assert!(
                        observation.outcome_correct,
                        "case {} failed on route {:?}",
                        case.case_id, observation.route
                    );
                    assert!(!observation.abstained);
                    assert!(!observation.freshness_error);
                }
                RepositoryExpectedOutcome::Abstain => {
                    assert!(observation.abstained, "case {} must abstain", case.case_id);
                    assert!(observation.outcome_correct);
                }
                RepositoryExpectedOutcome::Stale => {
                    // The fixture worktree is fresh, so the stale case reports
                    // a freshness error (the same contract as the rust corpus).
                    assert!(observation.freshness_error);
                }
            }
        }
    }

    // Every route pair is present.
    assert!(
        observations
            .iter()
            .any(|observation| observation.route == RepositoryRoute::PhaseC)
    );
    assert!(
        observations
            .iter()
            .any(|observation| observation.route == RepositoryRoute::CodeSpecialized)
    );

    // The fixture commit must be reachable from the corpus revision contract.
    assert_eq!(revision.len(), 40);

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
            observations:
                &'a [maestria_retrieval::repository_benchmark::RepositoryBenchmarkObservation],
        }
        fs::create_dir_all(&report_dir)?;
        let first = &observations[0];
        let report = Report {
            measurement_kind: "real_web_repository_code_index",
            corpus_id: &corpus.corpus_id,
            repository_revision: index.summary.commit_sha.as_str(),
            evaluation_date: &first.evaluation_date,
            index_generation: &first.index_generation,
            model_fingerprint: &first.model_fingerprint,
            route_config: &first.route_config,
            measurement_status: &first.measurement_status,
            observations: &observations,
        };
        fs::write(
            std::path::Path::new(&report_dir).join("web-repository-real.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(())
}
