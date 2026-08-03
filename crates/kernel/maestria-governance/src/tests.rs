use super::*;
use maestria_domain::{DomainEvent, DomainEventEnvelope, EvidenceId, MemoryCandidateId};

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
        security: maestria_domain::SecurityMetadata::default(),
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

    assert!(
        guard
            .check_read_containment(std::path::Path::new("/allowed/read/docs/note.md"))
            .is_ok()
    );
    assert!(
        guard
            .check_write_containment(std::path::Path::new("/allowed/write/output.md"))
            .is_ok()
    );
    assert!(
        guard
            .check_write_containment(std::path::Path::new("/allowed/read/docs/note.md"))
            .is_err()
    );
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

fn fixture_ocr_intent(
    disclosure: maestria_domain::OcrDisclosure,
) -> Result<maestria_domain::OcrIntent, Box<dyn std::error::Error>> {
    let identity = maestria_domain::OcrProviderIdentity::new(
        "fixture",
        "ocr",
        "v1",
        "sha256:provider",
        "prep-v1",
    )?;
    let source_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(b"pdf"))?;
    Ok(maestria_domain::OcrIntent::new(
        maestria_domain::ArtifactId::new(1),
        maestria_domain::BlobId::new(1),
        source_hash,
        [1],
        identity,
        disclosure,
    )?)
}

#[test]
fn ocr_risk_requires_low_for_local_no_retention_and_governs_other_disclosures()
-> Result<(), Box<dyn std::error::Error>> {
    let scope = ScopeGuard::new(Scope::new(
        vec![std::path::PathBuf::from("/data")],
        vec![std::path::PathBuf::from("/data")],
        vec![],
        vec![],
        false,
    ));
    let classifier = DefaultRiskClassifier;

    let local_no_retention = classifier.classify(
        &maestria_domain::MaestriaEffect::Ocr(maestria_domain::OcrEffect::new(fixture_ocr_intent(
            maestria_domain::OcrDisclosure::new(
                false,
                maestria_domain::OcrRetentionPolicy::NoRetention,
            ),
        )?)),
        &scope,
    );
    assert_eq!(local_no_retention, RiskClass::Low);

    let local_provider_defined = classifier.classify(
        &maestria_domain::MaestriaEffect::Ocr(maestria_domain::OcrEffect::new(fixture_ocr_intent(
            maestria_domain::OcrDisclosure::new(
                false,
                maestria_domain::OcrRetentionPolicy::ProviderDefined,
            ),
        )?)),
        &scope,
    );
    assert_eq!(local_provider_defined, RiskClass::Medium);

    let remote_no_retention = classifier.classify(
        &maestria_domain::MaestriaEffect::Ocr(maestria_domain::OcrEffect::new(fixture_ocr_intent(
            maestria_domain::OcrDisclosure::new(
                true,
                maestria_domain::OcrRetentionPolicy::NoRetention,
            ),
        )?)),
        &scope,
    );
    assert_eq!(remote_no_retention, RiskClass::High);
    Ok(())
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

#[test]
fn memory_promotion_denies_tainted_candidate() {
    let mut candidate = candidate_with_artifact(43, true);
    candidate.security.prompt_injection_risk = true;
    let decision = DefaultMemoryPromotionGate.evaluate(&MemoryPromotionRequest {
        candidate,
        user_approved: true,
    });
    assert!(matches!(decision, MemoryPromotionDecision::Deny { .. }));
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

    // QueryHarness with destructive commands must still be gated under ReadOnly.
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
