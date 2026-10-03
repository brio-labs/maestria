use std::fmt;

use serde::{Deserialize, Serialize};

use super::{Modality, SearchIntent};

/// The only governed route changes a search planner may record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchRouteDecision {
    UnsupportedIntent { intent: SearchIntent },
    UnsupportedModality { modality: Modality },
    MissingWebCapability,
    LocalTextFallback,
}

impl fmt::Display for SearchRouteDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedIntent { intent } => write!(
                formatter,
                "governed fallback to local text retrieval for unavailable {intent:?} intent"
            ),
            Self::UnsupportedModality { modality } => write!(
                formatter,
                "governed fallback to local text retrieval for unsupported {modality:?} modality"
            ),
            Self::MissingWebCapability => write!(
                formatter,
                "governed fallback to local text retrieval: web capability missing"
            ),
            Self::LocalTextFallback => {
                write!(formatter, "governed fallback to local text retrieval")
            }
        }
    }
}
