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
mod privacy;
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
pub use privacy::PrivacyExclusions;
pub use risk::{ClassifyRisk, DefaultRiskClassifier, PolicyDecision, RiskClass};
pub use scope::{ContainmentError, Scope, ScopeGuard};
pub use validation::{
    DefaultValidationGate, ValidationDecision, ValidationGate, ValidationRequest,
};

// ── metadata ────────────────────────────────────────────────────────

pub const GOVERNANCE_VERSION: &str = "0.1.0";

// ── legacy tests (preserved) ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        DomainEvent, DomainEventEnvelope, EvidenceId, MemoryCandidateId, TaskId, TaskStatus,
    };

    fn candidate_with_artifact(id: u64, has_evidence: bool) -> maestria_domain::MemoryCandidate {
        let mut evidence_ids = std::collections::BTreeSet::new();
        if has_evidence {
            evidence_ids.insert(EvidenceId::new(id));
        }

        maestria_domain::MemoryCandidate {
            id: MemoryCandidateId::new(id),
            claim_id: maestria_domain::ClaimId::new(id),
            evidence_ids,
            confidence_milli: 900,
        }
    }

    #[test]
    fn scope_guard_checks_read_write_paths() {
        let scope = Scope::new(
            vec![std::path::PathBuf::from("/allowed/read")],
            vec![std::path::PathBuf::from("/allowed/write")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);

        assert!(guard.allows_read(std::path::Path::new("/allowed/read/docs/note.md")));
        assert!(guard.allows_write(std::path::Path::new("/allowed/write/output.md")));
        assert!(!guard.allows_write(std::path::Path::new("/allowed/read/docs/note.md")));
        assert!(!guard.command_allowed("rm -rf /tmp"));
        assert!(guard.harness_allowed("shell"));
        assert!(guard.web_allowed());
    }

    #[test]
    fn approval_profile_changes_decision_without_domain_changes() {
        let scope = Scope::new(
            vec![std::path::PathBuf::from("/data")],
            vec![std::path::PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);

        let effect = maestria_domain::MaestriaEffect::PersistEvent {
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
            vec![std::path::PathBuf::from("/data")],
            vec![std::path::PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            false,
        );
        let guard = ScopeGuard::new(scope);
        let risky_effect =
            maestria_domain::MaestriaEffect::QueryHarness(maestria_domain::QueryHarnessRequest {
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
        let task = maestria_domain::Task {
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

    /// ReadOnly allows IndexFullText (rebuildable projection) but still gates risky effects.
    #[test]
    fn readonly_allows_full_text_index_but_gates_risky_effects() {
        let scope = Scope::new(
            vec![std::path::PathBuf::from("/data")],
            vec![std::path::PathBuf::from("/data")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        );
        let guard = ScopeGuard::new(scope);
        let gate = DefaultApprovalGate;

        // IndexFullText is a rebuildable projection — ReadOnly must allow it.
        let index_effect =
            maestria_domain::MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
                artifact_id: maestria_domain::ArtifactId::new(1),
                chunk_id: maestria_domain::ChunkId::new(10),
            });
        let index_req = ApprovalRequest {
            effect: &index_effect,
            profile: AutonomyProfile::ReadOnly,
            scope: &guard,
        };
        let index_decision = gate.decide(&index_req);
        assert!(
            index_decision.decision.is_allowed(),
            "IndexFullText must be allowed under ReadOnly (rebuildable projection)"
        );
        assert_eq!(index_decision.risk, RiskClass::Low);

        // QueryHarness with destructive commands must still be gated under ReadOnly.
        let harness_effect =
            maestria_domain::MaestriaEffect::QueryHarness(maestria_domain::QueryHarnessRequest {
                run_id: maestria_domain::HarnessRunId::new(1),
                task_id: None,
                generation: None,
                capability: "shell".into(),
                scope_id: maestria_domain::ScopeId::new(1),
                approval_id: None,
                command: "rm -rf /tmp".into(),
            });
        let harness_req = ApprovalRequest {
            effect: &harness_effect,
            profile: AutonomyProfile::ReadOnly,
            scope: &guard,
        };
        let harness_decision = gate.decide(&harness_req);
        assert!(
            !harness_decision.decision.is_allowed(),
            "QueryHarness with destructive command must be gated under ReadOnly"
        );
    }
}
