#![forbid(unsafe_code)]

//! Governance boundary for Maestria.
//!
//! This crate is intentionally side-effect free: it classifies and gates domain
//! intentions but performs no I/O. Runtime ports and adapter implementations are
//! expected to live elsewhere.

// ── modules ─────────────────────────────────────────────────────────

mod approval;
mod autonomy;
mod memory;
mod privacy;
mod risk;
mod scope;
mod validation;

// ── re-exports ──────────────────────────────────────────────────────

pub use approval::{ApprovalGate, ApprovalGateDecision, ApprovalRequest, DefaultApprovalGate};
pub use autonomy::AutonomyProfile;
pub use memory::{
    DefaultMemoryPromotionGate, MemoryPromotionDecision, MemoryPromotionGate,
    MemoryPromotionRequest,
};
pub use privacy::PrivacyExclusions;
pub use risk::{ClassifyRisk, DefaultRiskClassifier, PolicyDecision, RiskClass};
pub use scope::{ContainmentError, Scope, ScopeGuard};
pub use validation::{
    DefaultValidationGate, ValidationDecision, ValidationGate, ValidationRequest,
};

// ── metadata ────────────────────────────────────────────────────────

pub const GOVERNANCE_VERSION: &str = "0.1.0";

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
#[cfg(test)]
mod validation_tests;
