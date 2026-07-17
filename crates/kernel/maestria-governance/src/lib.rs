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
mod plan_validation;
mod privacy;
mod retrieval;
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
pub use plan_validation::{SearchCapabilities, SearchPlanValidationError, SearchPlanValidator};
pub use privacy::{PrivacyExclusions, SecretFinding, SecretKind, SecretScan, scan_secrets};
pub use retrieval::{RetrievalDecision, RetrievalSecurityPolicy};
pub use risk::{ClassifyRisk, DefaultRiskClassifier, PolicyDecision, RiskClass};
pub use scope::{ContainmentError, Scope, ScopeGuard};
pub use validation::{
    DefaultValidationGate, ValidationDecision, ValidationGate, ValidationRequest,
};

// ── metadata ────────────────────────────────────────────────────────

pub const GOVERNANCE_VERSION: &str = "0.6.0";

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod plan_validation_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod validation_tests;
