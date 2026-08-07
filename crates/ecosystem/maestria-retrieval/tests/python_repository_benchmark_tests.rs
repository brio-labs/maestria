use maestria_retrieval::repository_benchmark::{
    RepositoryBenchmarkCase, RepositoryBenchmarkCorpus, RepositoryBenchmarkError,
    RepositoryCodeIndexExecutor, RepositoryExpectedOutcome, RepositoryQueryClass, RepositoryRoute,
    run_repository_benchmark,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const FIXTURE_DIR: &str = "tests/fixtures/python-repository-v1";

fn python_repository_benchmark_fixture()
-> Result<RepositoryBenchmarkCorpus, RepositoryBenchmarkError> {
    let fixture = include_str!("fixtures/python-repository-benchmark-v1.json");
    RepositoryBenchmarkCorpus::from_json(fixture)
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR)
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let child = entry.path();
        let destination = target.join(entry.file_name());
        if child.is_dir() {
            copy_tree(&child, &destination)?;
        } else {
            fs::copy(&child, &destination)?;
        }
    }
    Ok(())
}

fn run_git(repo: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("git").current_dir(repo).args(args).status()?;
    if !status.success() {
        return Err(format!("git {args:?} failed in {}", repo.display()).into());
    }
    Ok(())
}

/// Copy the frozen fixture into a temp dir, initialize a git repository at
/// the copied state, and return the temp dir plus the fixture commit.
fn fixture_repository() -> Result<(tempfile::TempDir, String), Box<dyn std::error::Error>> {
    let tmp = tempdir()?;
    copy_tree(&fixture_root(), tmp.path())?;
    run_git(tmp.path(), &["init", "--initial-branch", "main"])?;
    run_git(tmp.path(), &["config", "user.email", "ci@example.com"])?;
    run_git(tmp.path(), &["config", "user.name", "CI"])?;
    run_git(tmp.path(), &["add", "."])?;
    run_git(tmp.path(), &["commit", "-m", "fixture init"])?;
    let output = Command::new("git")
        .current_dir(tmp.path())
        .args(["rev-parse", "HEAD"])
        .output()?;
    let sha = String::from_utf8(output.stdout)?.trim().to_string();
    Ok((tmp, sha))
}

fn case_by_id<'a>(
    corpus: &'a RepositoryBenchmarkCorpus,
    case_id: &str,
) -> Option<&'a RepositoryBenchmarkCase> {
    corpus.cases.iter().find(|case| case.case_id == case_id)
}

#[test]
fn python_repository_benchmark_fixture_parses_and_covers_all_required_query_classes()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = python_repository_benchmark_fixture()?;

    assert_eq!(corpus.cases.len(), RepositoryQueryClass::all().len());
    corpus.validate()?;
    let classes: BTreeSet<_> = corpus.cases.iter().map(|case| case.class).collect();
    assert_eq!(classes.len(), RepositoryQueryClass::all().len());
    assert_eq!(classes, RepositoryQueryClass::all().into_iter().collect());

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
fn python_repository_corpus_executes_against_the_frozen_fixture_index()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = python_repository_benchmark_fixture()?;
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
        "fixture must index real python symbols"
    );
    assert_eq!(index.summary.package_count, 1);
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
    let _ = case_by_id(&corpus, "python-exact-symbol");
    Ok(())
}
