use super::{
    LearnedSparseBenchmarkCase, LearnedSparseBenchmarkCorpus, LearnedSparseBenchmarkError,
    LearnedSparseBenchmarkObservation, LearnedSparseRoute,
};

/// Executes one frozen sparse case on one of the four benchmark routes.
pub trait LearnedSparseBenchmarkExecutor {
    fn observe(
        &self,
        case: LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Result<LearnedSparseBenchmarkObservation, LearnedSparseBenchmarkError>;
}

impl<F> LearnedSparseBenchmarkExecutor for F
where
    F: Fn(
        LearnedSparseBenchmarkCase,
        LearnedSparseRoute,
    ) -> Result<LearnedSparseBenchmarkObservation, LearnedSparseBenchmarkError>,
{
    fn observe(
        &self,
        case: LearnedSparseBenchmarkCase,
        route: LearnedSparseRoute,
    ) -> Result<LearnedSparseBenchmarkObservation, LearnedSparseBenchmarkError> {
        self(case, route)
    }
}

/// Execute every frozen case on every benchmark route.
///
/// The collected observations are validated against the corpus immediately:
/// a case or route missing from the matrix, or an observation that disagrees
/// with the corpus identity, fails here instead of surfacing later in the
/// comparison.
pub fn run_learned_sparse_benchmark<E: LearnedSparseBenchmarkExecutor>(
    corpus: &LearnedSparseBenchmarkCorpus,
    executor: &E,
) -> Result<Vec<LearnedSparseBenchmarkObservation>, LearnedSparseBenchmarkError> {
    corpus.validate()?;
    // The corpus declares the routes it evaluates (route_configurations);
    // an instance without a sparse lane evaluates the lexical and hybrid
    // routes only.
    let routes = corpus.route_configurations.keys().collect::<Vec<_>>();
    let mut observations = Vec::with_capacity(corpus.cases.len() * routes.len());
    for case in &corpus.cases {
        for route in &routes {
            observations.push(executor.observe(case.clone(), **route)?);
        }
    }
    super::comparison::validate_observations(corpus, &observations)?;
    Ok(observations)
}
