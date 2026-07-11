use maestria_domain::MaestriaEffect;

use crate::autonomy::AutonomyProfile;
use crate::risk::{ClassifyRisk, DefaultRiskClassifier, PolicyDecision, RiskClass};
use crate::scope::ScopeGuard;

/// A request submitted to the approval gate.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalRequest<'a> {
    pub effect: &'a MaestriaEffect,
    pub profile: AutonomyProfile,
    pub scope: &'a ScopeGuard,
}

/// Decision returned by an approval gate.
#[derive(Debug)]
pub struct ApprovalGateDecision {
    pub decision: PolicyDecision,
    pub risk: RiskClass,
}

/// Policy gate that decides whether an effect is allowed under a profile.
pub trait ApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision;
}

/// Default approval gate.
#[derive(Debug)]
pub struct DefaultApprovalGate;

impl DefaultApprovalGate {
    fn requires_approval_for(&self, profile: AutonomyProfile, risk: RiskClass) -> bool {
        matches!(
            (profile, risk),
            (AutonomyProfile::ReadOnly, RiskClass::Medium)
                | (AutonomyProfile::ReadOnly, RiskClass::High)
                | (AutonomyProfile::ReadOnly, RiskClass::Critical)
                | (AutonomyProfile::Assisted, RiskClass::High)
                | (AutonomyProfile::Assisted, RiskClass::Critical)
                | (AutonomyProfile::ScopedAutonomy, RiskClass::High)
                | (AutonomyProfile::StrictResearch, RiskClass::Critical)
                | (AutonomyProfile::TrustedWorkspace, RiskClass::Critical)
        )
    }

    fn denied(&self, profile: AutonomyProfile, risk: RiskClass) -> bool {
        matches!(
            (profile, risk),
            (AutonomyProfile::ReadOnly, RiskClass::Critical)
                | (AutonomyProfile::Assisted, RiskClass::Critical)
                | (AutonomyProfile::ScopedAutonomy, RiskClass::Critical)
                | (AutonomyProfile::StrictResearch, RiskClass::High)
        )
    }
}

impl ApprovalGate for DefaultApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision {
        let classifier = DefaultRiskClassifier;
        let risk = classifier.classify(request.effect, request.scope);
        let reason = match (request.profile, risk) {
            (AutonomyProfile::ReadOnly, RiskClass::Low) => {
                "read-only profile allows low-risk actions".to_string()
            }
            (AutonomyProfile::ReadOnly, _) => {
                "read-only profile blocks non-read operations without explicit approval".to_string()
            }
            (AutonomyProfile::Assisted, RiskClass::Low) => {
                "assisted profile allows low-risk actions".to_string()
            }
            (AutonomyProfile::Assisted, RiskClass::Medium) => {
                "assisted profile requires approval for medium risk actions".to_string()
            }
            (AutonomyProfile::Assisted, RiskClass::High | RiskClass::Critical) => {
                "assisted profile blocks high-risk actions without review".to_string()
            }
            (AutonomyProfile::ScopedAutonomy, RiskClass::Low) => {
                "scoped-autonomy profile allows low-risk actions".to_string()
            }
            (AutonomyProfile::ScopedAutonomy, RiskClass::Medium) => {
                "scoped-autonomy profile requires approval for medium risk".to_string()
            }
            (AutonomyProfile::ScopedAutonomy, RiskClass::High | RiskClass::Critical) => {
                "scoped-autonomy profile blocks high-risk actions".to_string()
            }
            (AutonomyProfile::StrictResearch, RiskClass::Medium | RiskClass::Low) => {
                "strict-research profile allows low/medium research actions".to_string()
            }
            (AutonomyProfile::StrictResearch, RiskClass::High) => {
                "strict-research profile requires approval for high risk actions".to_string()
            }
            (AutonomyProfile::StrictResearch, RiskClass::Critical) => {
                "strict-research profile blocks critical-risk actions".to_string()
            }
            (AutonomyProfile::TrustedWorkspace, RiskClass::High | RiskClass::Critical) => {
                "trusted-workspace profile requires approval for high risk actions".to_string()
            }
            (AutonomyProfile::TrustedWorkspace, _) => {
                "trusted-workspace profile allows non-critical actions".to_string()
            }
        };

        if self.denied(request.profile, risk) {
            ApprovalGateDecision {
                decision: PolicyDecision::Deny { reason },
                risk,
            }
        } else if self.requires_approval_for(request.profile, risk) {
            ApprovalGateDecision {
                decision: PolicyDecision::RequireApproval { reason },
                risk,
            }
        } else {
            ApprovalGateDecision {
                decision: PolicyDecision::Allow,
                risk,
            }
        }
    }
}
