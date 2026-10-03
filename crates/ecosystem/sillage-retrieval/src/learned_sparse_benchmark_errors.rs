use thiserror::Error;

use super::{LearnedSparseQueryClass, LearnedSparseRoute};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LearnedSparseBenchmarkError {
    #[error("invalid learned-sparse benchmark JSON: {0}")]
    InvalidJson(String),
    #[error("invalid learned-sparse benchmark corpus: {0}")]
    InvalidCorpus(String),
    #[error("learned-sparse benchmark is missing query class {0:?}")]
    MissingClass(LearnedSparseQueryClass),
    #[error("learned-sparse benchmark contains duplicate case {0}")]
    DuplicateCase(String),
    #[error("learned-sparse benchmark references unknown case {0}")]
    UnknownCase(String),
    #[error("invalid observation for case {case_id} on route {route:?}")]
    InvalidObservation {
        case_id: String,
        route: LearnedSparseRoute,
    },
    #[error("duplicate observation for case {case_id} on route {route:?}")]
    DuplicateObservation {
        case_id: String,
        route: LearnedSparseRoute,
    },
    #[error("missing observation for case {case_id} on route {route:?}")]
    MissingObservation {
        case_id: String,
        route: LearnedSparseRoute,
    },
    #[error("invalid learned-sparse identity: {0}")]
    InvalidIdentity(String),
    #[error("invalid learned-sparse measurement: {0}")]
    InvalidMeasurement(String),
    #[error("invalid learned-sparse promotion: {0}")]
    InvalidPromotion(String),
}
