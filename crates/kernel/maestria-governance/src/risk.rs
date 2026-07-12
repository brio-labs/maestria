use maestria_domain::MaestriaEffect;

use crate::scope::ScopeGuard;

/// Granularity of risk for an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskClass {
    Low,
    Medium,
    High,
    Critical,
}

/// Outcome of a policy gate decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireApproval { reason: String },
    Deny { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::RequireApproval { .. })
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }
}

/// Classify an effect by risk given the current scope.
pub trait ClassifyRisk {
    fn classify(&self, effect: &MaestriaEffect, scope: &ScopeGuard) -> RiskClass;
}

/// Default risk classifier based on effect variant and scope.
#[derive(Debug)]
pub struct DefaultRiskClassifier;

impl ClassifyRisk for DefaultRiskClassifier {
    fn classify(&self, effect: &MaestriaEffect, scope: &ScopeGuard) -> RiskClass {
        match effect {
            // Rebuildable projections: low-risk, no user-facing write or action authorization.
            MaestriaEffect::PersistEvent { .. }
            | MaestriaEffect::PersistState(_)
            | MaestriaEffect::StoreBlob(_)
            | MaestriaEffect::ParseArtifact(_)
            | MaestriaEffect::EmitDiagnostic(_)
            | MaestriaEffect::IndexFullText(_) => RiskClass::Low,
            MaestriaEffect::RunValidation(_)
            | MaestriaEffect::RequestApproval(_)
            | MaestriaEffect::IndexVector(_)
            | MaestriaEffect::UpdateGraph(_) => RiskClass::Medium,
            MaestriaEffect::FetchWeb(_) => {
                if scope.web_allowed() {
                    RiskClass::Medium
                } else {
                    RiskClass::High
                }
            }
            MaestriaEffect::QueryHarness(req) => {
                let command = req.command.to_lowercase();
                if command.starts_with("rm") || command.contains("delete") {
                    if scope.web_allowed() {
                        RiskClass::High
                    } else {
                        RiskClass::Critical
                    }
                } else {
                    RiskClass::Medium
                }
            }
        }
    }
}
