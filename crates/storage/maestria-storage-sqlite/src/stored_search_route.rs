use maestria_domain::SearchRouteDecision;
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search_plan::{StoredModality, StoredSearchIntent};
use maestria_ports::PortError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchRouteDecision {
    UnsupportedIntent { intent: StoredSearchIntent },
    UnsupportedModality { modality: StoredModality },
    MissingWebCapability,
    LocalTextFallback,
}

impl StoredSearchRouteDecision {
    pub(crate) fn from_domain(value: &SearchRouteDecision) -> Self {
        match value {
            SearchRouteDecision::UnsupportedIntent { intent } => Self::UnsupportedIntent {
                intent: StoredSearchIntent::from_domain(intent),
            },
            SearchRouteDecision::UnsupportedModality { modality } => Self::UnsupportedModality {
                modality: StoredModality::from_domain(modality),
            },
            SearchRouteDecision::MissingWebCapability => Self::MissingWebCapability,
            SearchRouteDecision::LocalTextFallback => Self::LocalTextFallback,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchRouteDecision, PortError> {
        Ok(match self {
            Self::UnsupportedIntent { intent } => SearchRouteDecision::UnsupportedIntent {
                intent: intent.try_into_domain()?,
            },
            Self::UnsupportedModality { modality } => SearchRouteDecision::UnsupportedModality {
                modality: modality.try_into_domain()?,
            },
            Self::MissingWebCapability => SearchRouteDecision::MissingWebCapability,
            Self::LocalTextFallback => SearchRouteDecision::LocalTextFallback,
        })
    }
}
