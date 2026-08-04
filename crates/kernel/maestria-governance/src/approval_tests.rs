use super::*;
use maestria_domain::{DomainEvent, DomainEventEnvelope};

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
        envelope: Box::new(DomainEventEnvelope {
            id: maestria_domain::EventId::new(1),
            sequence: maestria_domain::SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: maestria_domain::ArtifactId::new(1),
                title: "notes".to_string(),
                security: maestria_domain::SecurityMetadata::default(),
            },
        }),
    };
    let read_only = ApprovalRequest {
        effect: &effect,
        profile: AutonomyProfile::ReadOnly,
        scope: &guard,
        risk: RiskClass::Low,
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
            risk: RiskClass::Low,
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
            execution: maestria_domain::HarnessExecution::Fresh,
            capability: "shell".into(),
            scope_id: maestria_domain::ScopeId::new(1),
            command: "rm -rf /tmp".into(),
        });

    let request = ApprovalRequest {
        effect: &risky_effect,
        profile: AutonomyProfile::ScopedAutonomy,
        risk: RiskClass::Critical,
        scope: &guard,
    };
    let gate = DefaultApprovalGate;
    let decision = gate.decide(&request);

    assert!(matches!(
        decision.decision,
        PolicyDecision::Deny { .. } | PolicyDecision::RequireApproval { .. }
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

    let index_effect =
        maestria_domain::MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id: maestria_domain::ArtifactId::new(1),
            chunk_id: maestria_domain::ChunkId::new(10),
        });
    let index_req = ApprovalRequest {
        effect: &index_effect,
        risk: RiskClass::Low,
        profile: AutonomyProfile::ReadOnly,
        scope: &guard,
    };
    let index_decision = gate.decide(&index_req);
    assert!(
        index_decision.decision.is_allowed(),
        "IndexFullText must be allowed under ReadOnly (rebuildable projection)"
    );
    assert_eq!(index_decision.risk, RiskClass::Low);

    let harness_effect =
        maestria_domain::MaestriaEffect::QueryHarness(maestria_domain::QueryHarnessRequest {
            run_id: maestria_domain::HarnessRunId::new(1),
            task_id: None,
            execution: maestria_domain::HarnessExecution::Fresh,
            capability: "shell".into(),
            scope_id: maestria_domain::ScopeId::new(1),
            command: "rm -rf /tmp".into(),
        });
    let harness_req = ApprovalRequest {
        effect: &harness_effect,
        risk: RiskClass::Critical,
        profile: AutonomyProfile::ReadOnly,
        scope: &guard,
    };
    let harness_decision = gate.decide(&harness_req);
    assert!(
        !harness_decision.decision.is_allowed(),
        "QueryHarness with destructive command must be gated under ReadOnly"
    );
}

#[test]
fn approval_policy_exhaustively_covers_all_profile_risk_cells() {
    let scope = ScopeGuard::new(Scope::new(
        vec![std::path::PathBuf::from("/data")],
        vec![std::path::PathBuf::from("/data")],
        vec!["shell".into()],
        vec![],
        false,
    ));
    let low =
        maestria_domain::MaestriaEffect::IndexFullText(maestria_domain::IndexFullTextRequest {
            artifact_id: maestria_domain::ArtifactId::new(1),
            chunk_id: maestria_domain::ChunkId::new(1),
        });
    let medium =
        maestria_domain::MaestriaEffect::RunValidation(maestria_domain::RunValidationRequest {
            target: maestria_domain::ValidationTarget::Task(maestria_domain::TaskId::new(1)),
            validation_report_id: maestria_domain::ValidationReportId::new(1),
        });
    let high = maestria_domain::MaestriaEffect::FetchWeb(maestria_domain::FetchWebRequest {
        url: "https://example.test".into(),
        max_bytes: 1024,
        max_requests: 1,
        max_latency_ms: 1000,
        allowed_domains: vec!["example.test".into()],
        allowed_content_types: vec!["text/plain".into()],
    });
    let critical =
        maestria_domain::MaestriaEffect::QueryHarness(maestria_domain::QueryHarnessRequest {
            run_id: maestria_domain::HarnessRunId::new(1),
            task_id: None,
            execution: maestria_domain::HarnessExecution::Fresh,
            capability: "shell".into(),
            scope_id: maestria_domain::ScopeId::new(1),
            command: "rm -rf /tmp".into(),
        });
    let effects = [
        (&low, RiskClass::Low),
        (&medium, RiskClass::Medium),
        (&high, RiskClass::High),
        (&critical, RiskClass::Critical),
    ];
    let profiles = [
        AutonomyProfile::ReadOnly,
        AutonomyProfile::Assisted,
        AutonomyProfile::ScopedAutonomy,
        AutonomyProfile::StrictResearch,
        AutonomyProfile::TrustedWorkspace,
    ];
    let expected = [
        [
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::Deny,
        ],
        [
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::Deny,
        ],
        [
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::Deny,
        ],
        [
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::Deny,
        ],
        [
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::Allow,
            PolicyDecisionKind::RequireApproval,
            PolicyDecisionKind::RequireApproval,
        ],
    ];
    let gate = DefaultApprovalGate;
    for (profile_index, profile) in profiles.into_iter().enumerate() {
        for (risk_index, (effect, risk)) in effects.iter().enumerate() {
            let result = gate.decide(&ApprovalRequest {
                effect,
                profile,
                scope: &scope,
                risk: *risk,
            });
            assert_eq!(result.risk, *risk);
            let actual = match result.decision {
                PolicyDecision::Allow => PolicyDecisionKind::Allow,
                PolicyDecision::RequireApproval { .. } => PolicyDecisionKind::RequireApproval,
                PolicyDecision::Deny { .. } => PolicyDecisionKind::Deny,
            };
            assert_eq!(actual, expected[profile_index][risk_index]);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyDecisionKind {
    Allow,
    RequireApproval,
    Deny,
}
