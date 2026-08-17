//! Shared input validation helpers for port adapters.

use crate::PortError;

/// Rejects empty (whitespace-only) model labels with a typed error.
pub fn validate_model_label(model: &str, label: &str) -> Result<(), PortError> {
    if model.trim().is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "provider model is empty",
            source: format!("{label} model must contain a non-whitespace value"),
        });
    }
    Ok(())
}
