use maestria_domain::MaestriaEffect;

use crate::scope::Scope;

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
}

/// Classify an effect by risk given the current scope.
pub trait ClassifyRisk {
    fn classify(&self, effect: &MaestriaEffect, scope: &Scope) -> RiskClass;
}

/// Risk of a shell command under the current scope: destructive commands
/// escalate to High (web-enabled) or Critical; everything else is Medium.
fn classify_command_risk(command: &str, scope: &Scope) -> RiskClass {
    let command = command.to_lowercase();
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

/// Default risk classifier based on effect variant and scope.
#[derive(Debug)]
pub struct DefaultRiskClassifier;

impl ClassifyRisk for DefaultRiskClassifier {
    fn classify(&self, effect: &MaestriaEffect, scope: &Scope) -> RiskClass {
        match effect {
            // Rebuildable projections: low-risk, no user-facing write or action authorization.
            MaestriaEffect::PersistEvent { .. }
            | MaestriaEffect::PersistNotebookDraftBlob(_)
            | MaestriaEffect::ParseArtifact(_)
            | MaestriaEffect::IndexFullText(_) => RiskClass::Low,
            MaestriaEffect::Ocr(intent) => {
                if intent.disclosure().remote() {
                    RiskClass::High
                } else if matches!(
                    intent.disclosure().retention(),
                    maestria_domain::OcrRetentionPolicy::NoRetention
                ) {
                    RiskClass::Low
                } else {
                    RiskClass::Medium
                }
            }
            MaestriaEffect::SearchKnowledge(_) => RiskClass::Low,
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
            MaestriaEffect::QueryHarnessProposal(req) => classify_command_risk(&req.command, scope),
            MaestriaEffect::QueryHarness(req) => classify_command_risk(&req.command, scope),
        }
    }
}
