use super::{
    RepositoryBenchmarkCase, RepositoryBenchmarkCorpus, RepositoryBenchmarkError,
    RepositoryBenchmarkObservation, RepositoryRoute,
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
