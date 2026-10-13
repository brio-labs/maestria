#[path = "session_search_control.rs"]
mod control;
#[path = "session_search_deadline.rs"]
mod deadline;
#[path = "session_search_ingest.rs"]
mod ingest;
#[path = "session_search_models.rs"]
mod models;
#[path = "session_search_native.rs"]
mod native;
#[path = "session_search_operations.rs"]
mod operations;
#[path = "session_search_preparation_owner.rs"]
mod preparation_owner;
#[path = "session_search_prepare.rs"]
mod prepare;
#[path = "session_search_preview.rs"]
mod preview;
#[path = "session_search_preview_verification.rs"]
mod preview_verification;
#[path = "session_search_resources.rs"]
mod resources;
#[path = "session_search_retained.rs"]
mod retained;
#[path = "session_search_runtime.rs"]
mod runtime;
#[path = "session_search_source_admission.rs"]
mod source_admission;

use anyhow::Result;
use sillage_domain::SourceChoice;

pub use models::{SessionCoverage, SessionPreview, SessionSearchResponse, SessionSearchRuntime};
pub use preparation_owner::SessionPreparationOwner;

#[derive(Debug)]
pub struct StaleEvidence(&'static str);

impl std::fmt::Display for StaleEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for StaleEvidence {}

/// Typed context for recognized routine cancellation and deadline outcomes.
/// Any concrete worker/parser cause is retained in the error chain; a settlement
/// that supersedes successful work has no underlying failure cause. Consumers
/// classify this type, not rendered error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOperationSettlement {
    RoutineCancellation,
    DeadlineExceeded,
}

impl std::fmt::Display for SessionOperationSettlement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RoutineCancellation => formatter.write_str("session operation was cancelled"),
            Self::DeadlineExceeded => {
                formatter.write_str("session operation exceeded its selected deadline")
            }
        }
    }
}

impl std::error::Error for SessionOperationSettlement {}
impl SessionOperationSettlement {
    /// Returns a routine settlement only when the error chain has no lifecycle failure.
    pub fn classify_error(error: &anyhow::Error) -> Option<Self> {
        if error.downcast_ref::<SessionLifecycleFailure>().is_some() {
            return None;
        }
        error.downcast_ref::<Self>().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionLifecycleFailure {
    ControlResourceLink,
    OperationState,
    OwnerControlRegistry,
    SessionResourceState,
}

impl std::fmt::Display for SessionLifecycleFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ControlResourceLink => {
                formatter.write_str("session resource link mutex is poisoned")
            }
            Self::OperationState => {
                formatter.write_str("session operation tracker mutex is poisoned")
            }
            Self::OwnerControlRegistry => {
                formatter.write_str("session owner control registry mutex is poisoned")
            }
            Self::SessionResourceState => {
                formatter.write_str("session resource state mutex is poisoned")
            }
        }
    }
}

impl std::error::Error for SessionLifecycleFailure {}

pub(super) fn validate_session_choice(
    choice: &SourceChoice,
    retention_root_configured: bool,
) -> Result<()> {
    use sillage_domain::{
        ComputationTiming, DocumentaryRetentionChoice, SourceAvailability, SourceLocality,
        SourceRepresentation,
    };

    if choice.timing() != ComputationTiming::OnDemand {
        anyhow::bail!("selected Markdown preparation requires explicit on-demand computation");
    }
    if !matches!(choice.locality(), SourceLocality::LocalOnly)
        || !choice.remote_disclosures().is_empty()
    {
        anyhow::bail!("selected Markdown preparation supports local-only processing");
    }
    let representations = choice.representations();
    if representations.len() != 1 || representations[0] != SourceRepresentation::LexicalIndex {
        anyhow::bail!("selected Markdown preparation supports the canonical lexical index only");
    }
    if choice.availability() != SourceAvailability::Enabled {
        anyhow::bail!("selected source must be enabled before preparation");
    }
    if choice.documentary_retention() == DocumentaryRetentionChoice::RetainUntilExplicitlyPurged
        && !retention_root_configured
    {
        anyhow::bail!("documentary retention requires an explicitly configured owned store root");
    }
    Ok(())
}
#[cfg(test)]
mod settlement_classification_tests {
    use super::{SessionLifecycleFailure, SessionOperationSettlement};
    use anyhow::Result;

    #[test]
    fn lifecycle_failure_overrides_routine_settlement_classification() -> Result<()> {
        let settlement = SessionOperationSettlement::RoutineCancellation;
        let plain_error = anyhow::Error::new(settlement);
        if SessionOperationSettlement::classify_error(&plain_error) != Some(settlement) {
            anyhow::bail!("a plain typed routine settlement must remain routine");
        }

        let compound = plain_error.context(SessionLifecycleFailure::OperationState);
        if SessionOperationSettlement::classify_error(&compound).is_some() {
            anyhow::bail!("a lifecycle failure must prevent routine classification");
        }
        if compound.downcast_ref::<SessionOperationSettlement>() != Some(&settlement) {
            anyhow::bail!("lifecycle classification lost the underlying settlement cause");
        }
        let diagnostics = format!("{compound:#}");
        if !diagnostics.contains("session operation tracker mutex is poisoned")
            || !diagnostics.contains("session operation was cancelled")
        {
            anyhow::bail!("compound diagnostics omitted a typed failure");
        }
        Ok(())
    }
}
