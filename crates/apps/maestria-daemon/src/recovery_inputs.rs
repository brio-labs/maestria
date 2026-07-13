use maestria_domain::{DomainInput, KernelState};

use crate::full_text_recovery::pending_start_full_text;
use crate::parser_resume::pending_resume_parsers;
use crate::validation_recovery::pending_validations;

/// Typed container for pending recovery inputs computed from replayed kernel state.
#[derive(Debug, Clone)]
pub struct RecoveryInputs {
    pub resume_parsers: Vec<DomainInput>,
    pub start_full_text: Vec<DomainInput>,
    pub run_validations: Vec<DomainInput>,
}

/// Compute deterministic recovery inputs from replayed kernel state.
///
/// Returns parser recovery first, then full-text recovery, then task validation.
pub fn recovery_inputs(state: &KernelState) -> RecoveryInputs {
    RecoveryInputs {
        resume_parsers: pending_resume_parsers(state),
        start_full_text: pending_start_full_text(state),
        run_validations: pending_validations(state),
    }
}
