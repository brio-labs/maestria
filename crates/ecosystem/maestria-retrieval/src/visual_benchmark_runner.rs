use super::{
    VisualBenchmarkCase, VisualBenchmarkCorpus, VisualBenchmarkError, VisualBenchmarkObservation,
    VisualRoute,
};

/// Executes one frozen visual case on one benchmark route.
pub trait VisualBenchmarkExecutor {
    fn observe(
        &self,
        case: VisualBenchmarkCase,
        route: VisualRoute,
    ) -> Result<VisualBenchmarkObservation, VisualBenchmarkError>;
}

impl<F> VisualBenchmarkExecutor for F
where
    F: Fn(
        VisualBenchmarkCase,
        VisualRoute,
    ) -> Result<VisualBenchmarkObservation, VisualBenchmarkError>,
{
    fn observe(
        &self,
        case: VisualBenchmarkCase,
        route: VisualRoute,
    ) -> Result<VisualBenchmarkObservation, VisualBenchmarkError> {
        self(case, route)
    }
}

/// Execute every frozen visual case on both baseline and visual routes.
pub fn run_visual_benchmark<E: VisualBenchmarkExecutor>(
    corpus: &VisualBenchmarkCorpus,
    executor: &E,
) -> Result<Vec<VisualBenchmarkObservation>, VisualBenchmarkError> {
    corpus.validate()?;
    let mut observations = Vec::with_capacity(corpus.cases.len() * 2);
    for case in &corpus.cases {
        for route in [VisualRoute::TextLayout, VisualRoute::Visual] {
            observations.push(executor.observe(case.clone(), route)?);
        }
    }
    Ok(observations)
}
