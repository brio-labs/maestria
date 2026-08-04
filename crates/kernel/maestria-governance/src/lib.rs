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
/// - `privacy_exclusions`: path privacy exclusions.
/// - `prompt_injection`: prompt-injection classification.
/// - `retrieval`: module responsibility.
/// - `risk`: module responsibility.
/// - `scope`: module responsibility.
/// - `secret_scanning`: credential and secret classification.
/// - `validation`: task-completion validation.
/// - `version`: governance version metadata.
// ── modules ─────────────────────────────────────────────────────────
mod approval;
mod autonomy;
mod memory;
mod plan_validation;
mod privacy_exclusions;
mod prompt_injection;
mod retrieval;
mod risk;
mod scope;
mod secret_scanning;
mod validation;
mod version;

// ── re-exports ──────────────────────────────────────────────────────

pub use approval::{ApprovalGate, ApprovalGateDecision, ApprovalRequest, DefaultApprovalGate};
pub use autonomy::AutonomyProfile;
pub use memory::{
    DefaultMemoryPromotionGate, MemoryPromotionDecision, MemoryPromotionGate,
    MemoryPromotionRequest,
};
pub use plan_validation::{SearchCapabilities, SearchPlanValidationError, SearchPlanValidator};
pub use privacy_exclusions::PrivacyExclusions;
pub use prompt_injection::contains_prompt_injection_risk;
pub use retrieval::{
    RetrievalAuthorizationContext, RetrievalAuthorizationError, RetrievalDecision,
    RetrievalSecurityPolicy,
};
pub use risk::{ClassifyRisk, DefaultRiskClassifier, PolicyDecision, RiskClass};
pub use scope::{ContainmentError, Scope, ScopeGuard};
pub use secret_scanning::{SecretFinding, SecretKind, SecretScan, scan_secrets};
pub use validation::{
    DefaultValidationGate, ProposedCompletion, ValidationDecision, ValidationGate,
    ValidationRequest,
};

// ── metadata ────────────────────────────────────────────────────────

pub use version::GOVERNANCE_VERSION;

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod approval_tests;
#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod plan_validation_tests;
#[cfg(test)]
#[path = "privacy_exclusions_tests.rs"]
mod privacy_exclusions_tests;
#[cfg(test)]
#[path = "prompt_injection_tests.rs"]
mod prompt_injection_tests;
#[cfg(test)]
mod risk_tests;
#[cfg(test)]
mod scope_guard_tests;
#[cfg(test)]
#[path = "secret_scanning_tests.rs"]
mod secret_scanning_tests;
#[cfg(test)]
mod validation_tests;
