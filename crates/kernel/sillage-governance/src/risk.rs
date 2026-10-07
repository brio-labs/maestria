use sillage_domain::SillageEffect;

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
    fn classify(&self, effect: &SillageEffect, scope: &Scope) -> RiskClass;
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
    fn classify(&self, effect: &SillageEffect, scope: &Scope) -> RiskClass {
        match effect {
            // Rebuildable projections: low-risk, no user-facing write or action authorization.
            SillageEffect::PersistEvent { .. }
            | SillageEffect::PersistNotebookDraftBlob(_)
            | SillageEffect::ParseArtifact(_)
            | SillageEffect::IndexFullText(_) => RiskClass::Low,
            SillageEffect::Ocr(intent) => {
                if intent.disclosure().remote() {
                    RiskClass::High
                } else if matches!(
                    intent.disclosure().retention(),
                    sillage_domain::OcrRetentionPolicy::NoRetention
                ) {
                    RiskClass::Low
                } else {
                    RiskClass::Medium
                }
            }
            SillageEffect::SearchKnowledge(_) => RiskClass::Low,
            SillageEffect::RunValidation(_)
            | SillageEffect::RequestApproval(_)
            | SillageEffect::IndexArtifactVectors(_)
            | SillageEffect::UpdateGraph(_) => RiskClass::Medium,
            SillageEffect::FetchWeb(_) => {
                if scope.web_allowed() {
                    RiskClass::Medium
                } else {
                    RiskClass::High
                }
            }
            SillageEffect::QueryHarnessProposal(req) => classify_command_risk(&req.command, scope),
            SillageEffect::QueryHarness(req) => classify_command_risk(&req.command, scope),
        }
    }
}
