use maestria_domain::MaestriaEffect;

use crate::autonomy::AutonomyProfile;
use crate::risk::{PolicyDecision, RiskClass};
use crate::scope::ScopeGuard;

/// A request submitted to the approval gate.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalRequest<'a> {
    pub effect: &'a MaestriaEffect,
    pub profile: AutonomyProfile,
    pub scope: &'a ScopeGuard,
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
const POLICY_TABLE: [[PolicyCell; 4]; 5] = [
    [
        cell(
            PolicyAction::Allow,
            "read-only profile allows low-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "read-only profile requires approval for medium-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "read-only profile requires approval for high-risk actions",
        ),
        cell(
            PolicyAction::Deny,
            "read-only profile denies critical-risk actions",
        ),
    ],
    [
        cell(
            PolicyAction::Allow,
            "assisted profile allows low-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "assisted profile requires approval for medium-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "assisted profile requires approval for high-risk actions",
        ),
        cell(
            PolicyAction::Deny,
            "assisted profile denies critical-risk actions",
        ),
    ],
    [
        cell(
            PolicyAction::Allow,
            "scoped-autonomy profile allows low-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "scoped-autonomy profile requires approval for medium-risk actions",
        ),
        cell(
            PolicyAction::RequireApproval,
            "scoped-autonomy profile requires approval for high-risk actions",
        ),
        cell(
            PolicyAction::Deny,
            "scoped-autonomy profile denies critical-risk actions",
        ),
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
        AutonomyProfile::ReadOnly => 0,
        AutonomyProfile::Assisted => 1,
        AutonomyProfile::ScopedAutonomy => 2,
        AutonomyProfile::StrictResearch => 3,
        AutonomyProfile::TrustedWorkspace => 4,
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
