use maestria_domain::SearchIntent;
use thiserror::Error;
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LateInteractionBenchmarkError {
    #[error("invalid late-interaction JSON: {0}")]
    InvalidJson(String),
    #[error("invalid late-interaction corpus: {0}")]
    InvalidCorpus(&'static str),
    #[error("invalid late-interaction measurement: {0}")]
    InvalidMeasurement(&'static str),
    #[error("invalid late-interaction report: {0}")]
    InvalidReport(&'static str),
    #[error("late-interaction report identity does not match corpus")]
    ReportIdentityMismatch,
    #[error("late-interaction observation does not match corpus")]
    ObservationMismatch,
    #[error("late-interaction observations are missing class {0:?}")]
    MissingClass(SearchIntent),
    #[error("late-interaction observations are missing baseline for {0:?}")]
    MissingBaseline(SearchIntent),
    #[error("late-interaction observations are missing reranker for {0:?}")]
    MissingReranker(SearchIntent),
}
