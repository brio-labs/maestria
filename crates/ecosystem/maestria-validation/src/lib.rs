#![forbid(unsafe_code)]

//! Pure validation mechanisms for Maestria domain snapshots.
//!
//! This crate owns validation checks only. It does not decide policy, perform I/O,
//! or persist reports; callers provide an immutable domain snapshot and receive a
//! deterministic [`ValidationReport`].

pub mod runner;
mod search_provenance;
mod search_security;
pub mod search_validators;
pub mod types;

pub use types::SearchValidationContext;

pub use search_provenance::CandidateProvenanceValidator;
pub use search_security::{RetrievalSecurityValidator, SearchRegressionValidator};
pub use search_validators::{
    CitationAlignmentValidator, ConflictValidator, CoverageValidator, FreshnessValidator,
    SearchPlanValidator,
};
pub mod validators;

pub use runner::ValidationRunner;
pub use types::{Severity, ValidationCheck, ValidationContext, ValidationReport, Validator};
pub use validators::{
    CitationValidator, EvidenceExistenceValidator, HarnessRunValidator, MemoryValidator,
    TaskStateValidator,
};

#[cfg(test)]
#[path = "search_validator_tests.rs"]
mod search_validator_tests;
#[cfg(test)]
mod tests;
