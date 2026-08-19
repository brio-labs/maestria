use maestria_domain::MaestriaEffect;

use crate::autonomy::AutonomyProfile;
use crate::risk::{PolicyDecision, RiskClass};
use crate::scope::Scope;

/// A request submitted to the approval gate.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalRequest<'a> {
    pub effect: &'a MaestriaEffect,
    pub profile: AutonomyProfile,
    pub scope: &'a Scope,
    /// Risk is classified once by the caller and carried through this typed
    /// request into the exhaustive policy table.
    pub risk: RiskClass,
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

/// Admission policy: which effects bypass the approval gate entirely.
///
/// Implemented by the runtime, which owns the provider configuration that
/// some bypasses depend on (e.g. a vector effect with no embedding provider
/// can never execute, so it is admitted onto its degradation path instead of
/// producing a governance denial storm — issue #434).
pub trait AdmissionPolicy {
    fn bypasses_approval(&self, effect: &MaestriaEffect) -> bool;
}

/// Default approval gate.
#[derive(Debug)]
pub struct DefaultApprovalGate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyAction {
    Allow,
    RequireApproval,
    Deny,
}

#[derive(Debug, Clone, Copy)]
struct PolicyCell {
    action: PolicyAction,
    reason: &'static str,
}

const fn cell(action: PolicyAction, reason: &'static str) -> PolicyCell {
    PolicyCell { action, reason }
}

// Keep this table exhaustive: every autonomy profile × risk class pair has a
// single decision and reason. Risk classification happens before this lookup.
// The read-only, assisted, and scoped-autonomy profiles share one policy row
// (identical actions and reasons except for the profile name).
const POLICY_TABLE: [[PolicyCell; 4]; 3] = [
    [
        cell(PolicyAction::Allow, "low-risk actions are allowed"),
        cell(
            PolicyAction::RequireApproval,
            "medium-risk actions require approval",
        ),
        cell(
            PolicyAction::RequireApproval,
            "high-risk actions require approval",
        ),
        cell(PolicyAction::Deny, "critical-risk actions are denied"),
    ],
    [
        cell(
            PolicyAction::Allow,
            "strict-research profile allows low-risk actions",
        ),
        cell(
            PolicyAction::Allow,
            "strict-research profile allows medium-risk research actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "strict-research profile requires approval for high-risk actions",
        ),
        cell(
            PolicyAction::Deny,
            "strict-research profile denies critical-risk actions",
        ),
    ],
    [
        cell(
            PolicyAction::Allow,
            "trusted-workspace profile allows low-risk actions",
        ),
        cell(
            PolicyAction::Allow,
            "trusted-workspace profile allows medium-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "trusted-workspace profile requires approval for high-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "trusted-workspace profile requires approval for critical-risk actions",
        ),
    ],
];

fn profile_index(profile: AutonomyProfile) -> usize {
    match profile {
        AutonomyProfile::ReadOnly | AutonomyProfile::Assisted | AutonomyProfile::ScopedAutonomy => {
            0
        }
        AutonomyProfile::StrictResearch => 1,
        AutonomyProfile::TrustedWorkspace => 2,
    }
}

fn risk_index(risk: RiskClass) -> usize {
    match risk {
        RiskClass::Low => 0,
        RiskClass::Medium => 1,
        RiskClass::High => 2,
        RiskClass::Critical => 3,
    }
}

fn policy_cell(profile: AutonomyProfile, risk: RiskClass) -> PolicyCell {
    POLICY_TABLE[profile_index(profile)][risk_index(risk)]
}

impl ApprovalGate for DefaultApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision {
        let risk = request.risk;
        let cell = policy_cell(request.profile, risk);
        let decision = match cell.action {
            PolicyAction::Allow => PolicyDecision::Allow,
            PolicyAction::RequireApproval => PolicyDecision::RequireApproval {
                reason: cell.reason.to_string(),
            },
            PolicyAction::Deny => PolicyDecision::Deny {
                reason: cell.reason.to_string(),
            },
        };
        ApprovalGateDecision { decision, risk }
    }
}
