use maestria_domain::*;
use std::collections::BTreeSet;
#[path = "common/evidence.rs"]
mod common;
use common::{
    file_span_kind, promote_memory, register_artifact_and_claim, state_with_memory_candidate,
};

// ── Evidence provenance, relations, and memory lifecycle ──────────

#[test]
fn evidence_kind_preserves_provenance_and_triggers_claim_validation() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    let kind = EvidenceKind::CommandOutput {
        harness_run: HarnessRunId::new(77),
        stream: OutputStream::Stderr,
        blob: BlobId::new(55),
    };

    let output = state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: kind.clone(),
        excerpt: "stderr: assertion failed".to_string(),
        observed_at: LogicalTick::new(12),
        security: None,
    }))?;

    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind: event_kind,
                ..
            },
            ..
        }] if *evidence_id == EvidenceId::new(40)
            && *artifact_id == ArtifactId::new(1)
            && *claim_id == Some(ClaimId::new(20))
            && *event_kind == kind
    ));
    assert_eq!(
        output.effects,
        vec![
            MaestriaEffect::PersistEvent {
                envelope: output.events[0].clone(),
            },
            MaestriaEffect::RunValidation(RunValidationRequest {
                task_id: None,
                claim_id: Some(ClaimId::new(20)),
                validation_report_id: ValidationReportId::new(0),
            }),
        ]
    );

    let evidence =
        state
            .evidences
            .get(&EvidenceId::new(40))
            .ok_or(DomainError::MissingEvidence {
                id: EvidenceId::new(40),
            })?;
    assert_eq!(evidence.kind, kind);
    assert_eq!(evidence.excerpt, "stderr: assertion failed");
    assert_eq!(evidence.observed_at, LogicalTick::new(12));
    assert_eq!(
        state
            .claims
            .get(&ClaimId::new(20))
            .ok_or(DomainError::MissingClaim {
                id: ClaimId::new(20)
            })?
            .evidence_ids,
        BTreeSet::from([EvidenceId::new(40)])
    );
    Ok(())
}

fn assert_relation_created_with_valid_evidence(state: &mut KernelState) -> Result<(), DomainError> {
    let relation_output = state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: RelationId::new(70),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: Some(EvidenceId::new(40)),
        confidence_milli: 875,
        security: None,
    }))?;
    assert_eq!(
        state.relations.get(&RelationId::new(70)),
        Some(&Relation {
            id: RelationId::new(70),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(1)),
            evidence_id: Some(EvidenceId::new(40)),
            confidence_milli: 875,
            security: SecurityMetadata::default(),
        })
    );
    assert_eq!(
        relation_output.effects,
        vec![
            MaestriaEffect::PersistEvent {
                envelope: relation_output.events[0].clone(),
            },
            MaestriaEffect::UpdateGraph(UpdateGraphRequest {
                relation_id: RelationId::new(70),
            }),
        ]
    );
    Ok(())
}
fn assert_relation_created_without_evidence_skips_graph_update(
    state: &mut KernelState,
) -> Result<(), DomainError> {
    let relation_output = state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: RelationId::new(71),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: None,
        confidence_milli: 875,
        security: None,
    }))?;
    assert_eq!(
        state.relations.get(&RelationId::new(71)),
        Some(&Relation {
            id: RelationId::new(71),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(1)),
            evidence_id: None,
            confidence_milli: 875,
            security: SecurityMetadata::default(),
        })
    );
    assert_eq!(
        relation_output.effects,
        vec![MaestriaEffect::PersistEvent {
            envelope: relation_output.events[0].clone(),
        }]
    );
    Ok(())
}

fn assert_memory_candidate_created_with_evidence(
    state: &mut KernelState,
) -> Result<(), DomainError> {
    let candidate_output = state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(90),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40), EvidenceId::new(40)],
            confidence_milli: 720,
            security: None,
        },
    ))?;
    assert!(matches!(
        candidate_output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                ..
            },
            ..
        }] if *candidate_id == MemoryCandidateId::new(90)
            && *claim_id == ClaimId::new(20)
    ));
    let candidate = state
        .memory_candidates
        .get(&MemoryCandidateId::new(90))
        .ok_or(DomainError::MissingMemoryCandidate {
            id: MemoryCandidateId::new(90),
        })?;
    assert!(candidate.has_evidence());
    assert_eq!(candidate.claim_id, ClaimId::new(20));
    assert_eq!(
        candidate.evidence_ids,
        BTreeSet::from([EvidenceId::new(40)])
    );
    assert_eq!(candidate.confidence_milli, 720);
    Ok(())
}

#[test]
fn relation_and_memory_candidates_are_domain_owned_and_evidence_bound() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        security: None,
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_span_kind(),
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(12),
    }))?;

    assert_eq!(
        state
            .apply_input(DomainInput::CreateRelation(CreateRelationInput {
                relation_id: RelationId::new(99),
                security: None,
                source: RelationEndpoint::Claim(ClaimId::new(20)),
                kind: RelationKind::Supports,
                target: RelationEndpoint::Artifact(ArtifactId::new(1)),
                evidence_id: Some(EvidenceId::new(404)),
                confidence_milli: 875,
            }))
            .err(),
        Some(DomainError::MissingEvidence {
            id: EvidenceId::new(404)
        })
    );

    assert_relation_created_without_evidence_skips_graph_update(&mut state)?;
    assert_relation_created_with_valid_evidence(&mut state)?;

    assert!(matches!(
        state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(91),
                claim_id: ClaimId::new(20),
                evidence_ids: Vec::new(),
                confidence_milli: 720,
                security: None,
            },
        )),
        Err(DomainError::EvidenceRequired {
            kind: "memory_candidate",
            id: 91,
        })
    ));

    assert_memory_candidate_created_with_evidence(&mut state)?;
    Ok(())
}

#[test]
fn promote_memory_creates_active_memory_from_candidate() -> Result<(), DomainError> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;

    let output = state.apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
        memory_id: MemoryId::new(100),
        candidate_id: MemoryCandidateId::new(90),
    }))?;

    let memory = state
        .memories
        .get(&MemoryId::new(100))
        .ok_or(DomainError::MissingMemory {
            id: MemoryId::new(100),
        })?;
    assert_eq!(memory.candidate_id, MemoryCandidateId::new(90));
    assert_eq!(memory.claim_id, ClaimId::new(20));
    assert_eq!(memory.evidence_ids, BTreeSet::from([EvidenceId::new(40)]));
    assert_eq!(memory.status, MemoryStatus::Active);
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
                ..
            },
            ..
        }] if *memory_id == MemoryId::new(100)
            && *candidate_id == MemoryCandidateId::new(90)
    ));
    assert_eq!(
        output.effects,
        vec![MaestriaEffect::PersistEvent {
            envelope: output.events[0].clone(),
        }]
    );
    Ok(())
}

#[test]
fn promote_memory_rejects_missing_candidate() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    assert_eq!(
        state
            .apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
                memory_id: MemoryId::new(100),
                candidate_id: MemoryCandidateId::new(404),
            }))
            .err(),
        Some(DomainError::MissingMemoryCandidate {
            id: MemoryCandidateId::new(404),
        })
    );
    Ok(())
}

#[test]
fn contradict_memory_marks_memory_contradicted() -> Result<(), DomainError> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(91),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 650,
            security: None,
        },
    ))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

    let output = state.apply_input(DomainInput::ContradictMemory(ContradictMemoryInput {
        memory_id: MemoryId::new(100),
        contradicting_candidate_id: MemoryCandidateId::new(91),
    }))?;

    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Contradicted
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            },
            ..
        }] if *memory_id == MemoryId::new(100)
            && *contradicting_candidate_id == MemoryCandidateId::new(91)
    ));
    Ok(())
}

#[test]
fn deprecate_memory_marks_memory_deprecated() -> Result<(), DomainError> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

    let output = state.apply_input(DomainInput::DeprecateMemory(DeprecateMemoryInput {
        memory_id: MemoryId::new(100),
    }))?;

    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Deprecated
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryDeprecated { memory_id },
            ..
        }] if *memory_id == MemoryId::new(100)
    ));
    Ok(())
}

#[test]
fn supersede_memory_marks_memory_superseded() -> Result<(), DomainError> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(91),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 650,
            security: Some(SecurityMetadata {
                trust_zone: TrustZone::Verified,
                authority: Authority::User,
                ..SecurityMetadata::default()
            }),
        },
    ))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;
    promote_memory(&mut state, MemoryId::new(101), MemoryCandidateId::new(91))?;

    let output = state.apply_input(DomainInput::SupersedeMemory(SupersedeMemoryInput {
        memory_id: MemoryId::new(100),
        by_memory_id: MemoryId::new(101),
    }))?;

    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Superseded
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            },
            ..
        }] if *memory_id == MemoryId::new(100)
            && *by_memory_id == MemoryId::new(101)
    ));
    Ok(())
}

#[test]
fn record_validation_report_emits_informational_event() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(50),
        title: "Validate answer".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;

    let output = state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id: ValidationReportId::new(80),
            task_id: Some(TaskId::new(50)),
            passed: true,
            warnings: vec!["minor style warning".to_string()],
        },
    ))?;

    assert_eq!(output.events.len(), 1);
    if let DomainEvent::ValidationReportCreated {
        report_id,
        task_id,
        passed,
        warnings,
    } = &output.events[0].event
    {
        assert_eq!(*report_id, ValidationReportId::new(80));
        assert_eq!(*task_id, Some(TaskId::new(50)));
        assert!(*passed);
        assert_eq!(warnings, &vec!["minor style warning".to_string()]);
    } else {
        panic!("expected validation report created event");
    }
    assert!(
        output
            .effects
            .iter()
            .any(|effect| matches!(effect, MaestriaEffect::PersistEvent { .. }))
    );
    Ok(())
}

// ── Evidence idempotency / duplicate rejection ───────────────────

#[test]
fn record_evidence_duplicate_is_idempotent() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;

    let evidence_input = RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::CommandOutput {
            harness_run: HarnessRunId::new(1),
            stream: OutputStream::Stdout,
            blob: BlobId::new(99),
        },
        excerpt: "output text".to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    };

    let output1 = state.apply_input(DomainInput::RecordEvidence(evidence_input.clone()))?;
    assert!(
        output1
            .events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::EvidenceRecorded { .. }))
    );

    let output2 = state.apply_input(DomainInput::RecordEvidence(evidence_input))?;
    assert!(
        output2.events.is_empty(),
        "duplicate evidence produces no events"
    );
    assert!(
        output2.effects.is_empty(),
        "duplicate evidence produces no effects"
    );

    Ok(())
}

#[test]
fn record_evidence_rejects_mismatched_duplicate() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::CommandOutput {
            harness_run: HarnessRunId::new(1),
            stream: OutputStream::Stdout,
            blob: BlobId::new(99),
        },
        excerpt: "original".to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;

    let err = state
        .apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(1),
                stream: OutputStream::Stdout,
                blob: BlobId::new(99),
            },
            excerpt: "different excerpt".to_string(),
            observed_at: LogicalTick::new(1),
            security: None,
        }))
        .expect_err("mismatched evidence must error");

    assert!(matches!(err, DomainError::DuplicateId { kind, id: 40 } if kind == "evidence"));
    Ok(())
}

#[test]
fn record_evidence_rejects_observed_at_mismatch() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::CommandOutput {
            harness_run: HarnessRunId::new(1),
            stream: OutputStream::Stdout,
            blob: BlobId::new(99),
        },
        excerpt: "same excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: None,
    }))?;

    let err = state
        .apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(1),
                stream: OutputStream::Stdout,
                blob: BlobId::new(99),
            },
            excerpt: "same excerpt".to_string(),
            observed_at: LogicalTick::new(2),
            security: None,
        }))
        .expect_err("observed_at mismatch must error");

    assert!(
        matches!(err, DomainError::DuplicateId { kind, id: 40 } if kind == "evidence"),
        "expected DuplicateId for evidence, got {:?}",
        err
    );
    Ok(())
}

// ── Memory proposal (atomic claim + candidate) ────────────────────
