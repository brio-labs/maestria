#![forbid(unsafe_code)]

//! Governance boundary for Maestria.
//!
//! This crate is intentionally side-effect free: it classifies and gates domain
//! intentions but performs no I/O. Runtime ports and adapter implementations are
//! expected to live elsewhere.

/// Responsibility map:
/// - `approval`: module responsibility.
/// - `autonomy`: module responsibility.
/// - `memory`: module responsibility.
/// - `plan_validation`: module responsibility.
/// - `privacy`: module responsibility.
/// - `retrieval`: module responsibility.
/// - `risk`: module responsibility.
/// - `scope`: module responsibility.
/// - `validation`: module responsibility.
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
pub use privacy::{
    PrivacyExclusions, SecretFinding, SecretKind, SecretScan, contains_prompt_injection_risk,
    scan_secrets,
};
pub use retrieval::{RetrievalDecision, RetrievalSecurityPolicy};
pub use risk::{ClassifyRisk, DefaultRiskClassifier, PolicyDecision, RiskClass};
pub use scope::{ContainmentError, Scope, ScopeGuard};
pub use validation::{
    DefaultValidationGate, ValidationDecision, ValidationGate, ValidationRequest,
};

// ── metadata ────────────────────────────────────────────────────────

pub const GOVERNANCE_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod plan_validation_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod validation_tests;
