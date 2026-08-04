use std::fmt;

use maestria_domain::{CorpusSnapshotId, IndexGenerationId, Modality, SearchIntent, SearchStage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchPlanValidationError {
    IntentMismatch {
        declared: SearchIntent,
        classified: SearchIntent,
    },
    UnsupportedIntent(SearchIntent),
    UnsupportedStage(SearchStage),
    UnsupportedModality(Modality),
    SnapshotUnavailable(CorpusSnapshotId),
    GenerationUnavailable(IndexGenerationId),
    ScopeDenied,
    TooManyScopes {
        requested: usize,
        allowed: u32,
    },
    FreshnessUnsupported,
    BudgetExceeded {
        budget: &'static str,
        requested: u64,
        allowed: u64,
    },
    SecurityCapabilityMissing(&'static str),
    WebCapabilityMissing,
}

impl fmt::Display for SearchPlanValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntentMismatch {
                declared,
                classified,
            } => write!(
                f,
                "declared search intent {declared:?} does not match deterministic classification {classified:?}"
            ),
            Self::UnsupportedIntent(intent) => write!(f, "unsupported search intent: {intent:?}"),
            Self::UnsupportedStage(stage) => write!(f, "unsupported search stage: {stage:?}"),
            Self::UnsupportedModality(modality) => {
                write!(f, "unsupported search modality: {modality:?}")
            }
            Self::SnapshotUnavailable(snapshot) => {
                write!(f, "corpus snapshot {} is unavailable", snapshot.value())
            }
            Self::GenerationUnavailable(generation) => {
                write!(f, "index generation {} is unavailable", generation.value())
            }
            Self::ScopeDenied => write!(f, "search plan scope is not allowed by policy"),
            Self::TooManyScopes { requested, allowed } => {
                write!(
                    f,
                    "search plan requests {requested} scopes; maximum is {allowed}"
                )
            }
            Self::FreshnessUnsupported => write!(f, "freshness requirement is unsupported"),
            Self::BudgetExceeded {
                budget,
                requested,
                allowed,
            } => write!(
                f,
                "{budget} budget requests {requested}; capability allows {allowed}"
            ),
            Self::SecurityCapabilityMissing(capability) => {
                write!(
                    f,
                    "required security capability is unavailable: {capability}"
                )
            }
            Self::WebCapabilityMissing => write!(f, "web retrieval capability is unavailable"),
        }
    }
}

impl std::error::Error for SearchPlanValidationError {}
