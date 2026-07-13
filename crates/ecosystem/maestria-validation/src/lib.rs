#![forbid(unsafe_code)]

//! Pure validation mechanisms for Maestria domain snapshots.
//!
//! This crate owns validation checks only. It does not decide policy, perform I/O,
//! or persist reports; callers provide an immutable domain snapshot and receive a
//! deterministic [`ValidationReport`].

pub mod runner;
pub mod types;
pub mod validators;

pub use runner::ValidationRunner;
pub use types::{ValidationCheck, ValidationContext, ValidationReport, Validator};
pub use validators::{
    CitationValidator, EvidenceExistenceValidator, HarnessRunValidator, MemoryValidator,
    TaskStateValidator,
};

#[cfg(test)]
mod tests;
