use super::{
    LateInteractionBenchmarkCase, LateInteractionBenchmarkCorpus, LateInteractionBenchmarkError,
    LateInteractionObservation, LateInteractionRoute,
};

/// Executes one frozen Stage A case on one route.
pub trait LateInteractionBenchmarkExecutor {
    fn observe(
        &self,
        case: LateInteractionBenchmarkCase,
        route: LateInteractionRoute,
    ) -> Result<LateInteractionObservation, LateInteractionBenchmarkError>;
}

impl<F> LateInteractionBenchmarkExecutor for F
where
    F: Fn(
        LateInteractionBenchmarkCase,
        LateInteractionRoute,
    ) -> Result<LateInteractionObservation, LateInteractionBenchmarkError>,
{
    fn observe(
        &self,
        case: LateInteractionBenchmarkCase,
        route: LateInteractionRoute,
    ) -> Result<LateInteractionObservation, LateInteractionBenchmarkError> {
        self(case, route)
    }
}

pub fn run_late_interaction_stage_a<E: LateInteractionBenchmarkExecutor>(
    corpus: &LateInteractionBenchmarkCorpus,
    executor: &E,
) -> Result<Vec<LateInteractionObservation>, LateInteractionBenchmarkError> {
    corpus.validate()?;
    let mut observations =
        Vec::with_capacity(corpus.cases.len() * LateInteractionRoute::all_stage_a().len());
    for case in &corpus.cases {
        for route in LateInteractionRoute::all_stage_a() {
            let observation = executor.observe(case.clone(), route)?;
            if observation.corpus_id != corpus.corpus_id
                || observation.corpus_revision != corpus.revision
                || observation.case_id != case.case_id
                || observation.query_class != case.query_class
                || observation.route != route
            {
                return Err(LateInteractionBenchmarkError::ObservationMismatch);
            }
            observation.quality.validate()?;
            observation
                .measurement_status
                .validate()
                .map_err(LateInteractionBenchmarkError::InvalidMeasurement)?;
            observations.push(observation);
        }
    }
    Ok(observations)
}
