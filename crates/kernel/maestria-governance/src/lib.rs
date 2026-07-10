#![forbid(unsafe_code)]

//! Governance boundary for Maestria.
//!
//! This crate is intentionally side-effect free: it classifies and gates domain
//! intentions but performs no I/O. Runtime ports and adapter implementations are
//! expected to live elsewhere.

use std::path::{Path, PathBuf};

use maestria_domain::{MaestriaEffect, MemoryCandidate, Task};

pub const GOVERNANCE_VERSION: &str = "0.1.0";

//
// Scope and policy surface
//

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scope {
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    allowed_harnesses: Vec<String>,
    blocked_commands: Vec<String>,
    web_allowed: bool,
}

impl Scope {
    pub fn new(
        read_roots: Vec<PathBuf>,
        write_roots: Vec<PathBuf>,
        allowed_harnesses: Vec<String>,
        blocked_commands: Vec<String>,
        web_allowed: bool,
    ) -> Self {
        Self {
            read_roots,
            write_roots,
            allowed_harnesses,
            blocked_commands,
            web_allowed,
        }
    }

    pub fn allows_read(&self, path: &Path) -> bool {
        self.read_roots.iter().any(|root| path.starts_with(root))
            || self.write_roots.iter().any(|root| path.starts_with(root))
    }

    pub fn allows_write(&self, path: &Path) -> bool {
        self.write_roots.iter().any(|root| path.starts_with(root))
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        let command = command.trim().to_lowercase();
        if command.is_empty() {
            return false;
        }
        !self.blocked_commands.iter().any(|entry| {
            let entry = entry.as_str().trim().to_lowercase();
            command == entry || command.starts_with(&format!("{entry} "))
        })
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.allowed_harnesses.iter().any(|entry| entry == harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.web_allowed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeGuard {
    scope: Scope,
}

impl ScopeGuard {
    pub fn new(scope: Scope) -> Self {
        Self { scope }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn allows_read(&self, path: &Path) -> bool {
        self.scope.allows_read(path)
    }

    pub fn allows_write(&self, path: &Path) -> bool {
        self.scope.allows_write(path)
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        self.scope.command_allowed(command)
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.scope.harness_allowed(harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.scope.web_allowed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskClass {
    Low,
    Medium,
    High,
    Critical,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomyProfile {
    ReadOnly,
    Assisted,
    ScopedAutonomy,
    StrictResearch,
    TrustedWorkspace,
}

#[derive(Debug, Clone, Copy)]
pub struct ApprovalRequest<'a> {
    pub effect: &'a MaestriaEffect,
    pub profile: AutonomyProfile,
    pub scope: &'a ScopeGuard,
}

pub trait ClassifyRisk {
    fn classify(&self, effect: &MaestriaEffect, scope: &ScopeGuard) -> RiskClass;
}

#[derive(Debug)]
pub struct DefaultRiskClassifier;

impl ClassifyRisk for DefaultRiskClassifier {
    fn classify(&self, effect: &MaestriaEffect, scope: &ScopeGuard) -> RiskClass {
        match effect {
            MaestriaEffect::PersistEvent { .. }
            | MaestriaEffect::PersistState(_)
            | MaestriaEffect::StoreBlob(_)
            | MaestriaEffect::ParseArtifact(_)
            | MaestriaEffect::EmitDiagnostic(_) => RiskClass::Low,
            MaestriaEffect::RunValidation(_)
            | MaestriaEffect::RequestApproval(_)
            | MaestriaEffect::IndexFullText(_)
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

#[derive(Debug)]
pub struct ApprovalGateDecision {
    pub decision: PolicyDecision,
    pub risk: RiskClass,
}

pub trait ApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision;
}

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

#[derive(Debug)]
pub struct ValidationRequest {
    pub task: Task,
    pub validation_report_present: bool,
    pub had_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDecision {
    AllowCompletion,
    BlockedByMissingValidation { reason: String },
    BlockedByPolicy { reason: String },
}

pub trait ValidationGate {
    fn evaluate(&self, request: &ValidationRequest) -> ValidationDecision;
}

#[derive(Debug)]
pub struct DefaultValidationGate {
    allow_warnings: bool,
}

impl DefaultValidationGate {
    pub const fn new(allow_warnings: bool) -> Self {
        Self { allow_warnings }
    }
}

impl ValidationGate for DefaultValidationGate {
    fn evaluate(&self, request: &ValidationRequest) -> ValidationDecision {
        if !request.validation_report_present {
            return ValidationDecision::BlockedByMissingValidation {
                reason: "task completion requires validation report".to_string(),
            };
        }

        if request.task.status.is_completion() {
            if request.had_warning && !self.allow_warnings {
                return ValidationDecision::BlockedByPolicy {
                    reason: "warnings are blocked in this policy".to_string(),
                };
            }
            ValidationDecision::AllowCompletion
        } else {
            ValidationDecision::BlockedByPolicy {
                reason: format!(
                    "task status {:?} is not completion state",
                    request.task.status
                ),
            }
        }
    }
}

#[derive(Debug)]
pub struct MemoryPromotionRequest {
    pub candidate: MemoryCandidate,
    pub user_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryPromotionDecision {
    Promote,
    RequireEvidence { reason: String },
    RequireReview { reason: String },
    Deny { reason: String },
}

pub trait MemoryPromotionGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision;
}

#[derive(Debug)]
pub struct DefaultMemoryPromotionGate;

impl MemoryPromotionGate for DefaultMemoryPromotionGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision {
        if !request.candidate.has_evidence() {
            return MemoryPromotionDecision::RequireEvidence {
                reason: "memory candidate must contain at least one evidence id".to_string(),
            };
        }

        if request.candidate.confidence_milli < 500 {
            return MemoryPromotionDecision::RequireReview {
                reason: "low confidence memory candidate".to_string(),
            };
        }

        if request.user_approved {
            MemoryPromotionDecision::Promote
        } else {
            MemoryPromotionDecision::RequireReview {
                reason: "user approval required for promotion".to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        DomainEvent, DomainEventEnvelope, EvidenceId, MemoryCandidateId, TaskId, TaskStatus,
    };
    fn candidate_with_artifact(id: u64, has_evidence: bool) -> MemoryCandidate {
        let mut evidence_ids = std::collections::BTreeSet::new();
        if has_evidence {
            evidence_ids.insert(EvidenceId::new(id));
        }

        MemoryCandidate {
            id: MemoryCandidateId::new(id),
            claim_id: maestria_domain::ClaimId::new(id),
            evidence_ids,
            confidence_milli: 900,
        }
    }

    #[test]
    fn scope_guard_checks_read_write_paths() {
        let scope = Scope::new(
            vec![PathBuf::from("/allowed/read")],
            vec![PathBuf::from("/allowed/write")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);

        assert!(guard.allows_read(Path::new("/allowed/read/docs/note.md")));
        assert!(guard.allows_write(Path::new("/allowed/write/output.md")));
        assert!(!guard.allows_write(Path::new("/allowed/read/docs/note.md")));
        assert!(!guard.command_allowed("rm -rf /tmp"));
        assert!(guard.harness_allowed("shell"));
        assert!(guard.web_allowed());
    }

    #[test]
    fn approval_profile_changes_decision_without_domain_changes() {
        let scope = Scope::new(
            vec![PathBuf::from("/data")],
            vec![PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);

        let effect = MaestriaEffect::PersistEvent {
            envelope: DomainEventEnvelope {
                id: maestria_domain::EventId::new(1),
                sequence: maestria_domain::SequenceNumber::new(1),
                event: DomainEvent::ArtifactRegistered {
                    artifact_id: maestria_domain::ArtifactId::new(1),
                    title: "notes".to_string(),
                },
            },
        };
        let read_only = ApprovalRequest {
            effect: &effect,
            profile: AutonomyProfile::ReadOnly,
            scope: &guard,
        };
        let assisted = ApprovalRequest {
            profile: AutonomyProfile::Assisted,
            ..read_only
        };

        let gate = DefaultApprovalGate;
        let read_only_decision = gate.decide(&read_only);
        let assisted_decision = gate.decide(&assisted);

        assert!(read_only_decision.decision.is_allowed());
        assert!(assisted_decision.decision.is_allowed());
        assert!(read_only_decision.risk <= assisted_decision.risk);
        assert!(matches!(
            gate.decide(&ApprovalRequest {
                profile: AutonomyProfile::StrictResearch,
                effect: &effect,
                scope: &guard,
            })
            .decision,
            PolicyDecision::Allow
        ));
    }

    #[test]
    fn risky_effects_require_approval_gate() {
        let scope = Scope::new(
            vec![PathBuf::from("/data")],
            vec![PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            false,
        );
        let guard = ScopeGuard::new(scope);
        let risky_effect = MaestriaEffect::QueryHarness(maestria_domain::QueryHarnessRequest {
            run_id: maestria_domain::HarnessRunId::new(1),
            task_id: None,
            generation: None,
            capability: "shell".into(),
            scope_id: maestria_domain::ScopeId::new(1),
            approval_id: None,
            command: "rm -rf /tmp".into(),
        });

        let request = ApprovalRequest {
            effect: &risky_effect,
            profile: AutonomyProfile::ScopedAutonomy,
            scope: &guard,
        };
        let gate = DefaultApprovalGate;
        let decision = gate.decide(&request);

        assert!(matches!(
            decision.decision,
            PolicyDecision::Deny { .. } | PolicyDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn validation_gate_requires_report() {
        let task = Task {
            id: TaskId::new(12),
            title: "example".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            status: TaskStatus::CompletedVerified,
            validation_report_id: Some(maestria_domain::ValidationReportId::new(1)),
            artifact_ids: Default::default(),
            evidence_ids: Default::default(),
        };

        let gate = DefaultValidationGate::new(true);
        let decision = gate.evaluate(&ValidationRequest {
            task,
            validation_report_present: false,
            had_warning: false,
        });
        assert!(matches!(
            decision,
            ValidationDecision::BlockedByMissingValidation { .. }
        ));
    }

    #[test]
    fn memory_promotion_gate_requires_evidence() {
        let candidate = candidate_with_artifact(42, false);
        let request = MemoryPromotionRequest {
            candidate,
            user_approved: true,
        };

        let decision = DefaultMemoryPromotionGate.evaluate(&request);
        assert!(matches!(
            decision,
            MemoryPromotionDecision::RequireEvidence { .. }
        ));
    }
}
