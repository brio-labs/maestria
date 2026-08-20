use maestria_domain::*;
use std::collections::BTreeSet;
#[path = "common/assertions.rs"]
mod assertions;
#[path = "common/evidence.rs"]
mod common;

use assertions::require_error;
use common::register_artifact_and_claim;

// ── Evidence provenance and record-time validation ────────────────

#[test]
fn evidence_kind_preserves_provenance_and_triggers_claim_validation()
-> Result<(), Box<dyn std::error::Error>> {
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
                envelope: Box::new(output.events[0].clone()),
            },
            MaestriaEffect::RunValidation(RunValidationRequest {
                target: ValidationTarget::Claim(ClaimId::new(20)),
                validation_report_id: ValidationReportId::new(1),
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

#[test]
fn record_evidence_duplicate_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
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
fn record_evidence_rejects_mismatched_duplicate() -> Result<(), Box<dyn std::error::Error>> {
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

    let err = require_error(
        state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
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
        })),
        "mismatched evidence must error",
    )?;

    assert!(matches!(
        err,
        DomainError::DuplicateEvidence { id } if id.value() == 40
    ));
    Ok(())
}

#[test]
fn record_evidence_rejects_observed_at_mismatch() -> Result<(), Box<dyn std::error::Error>> {
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

    let err = require_error(
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
            observed_at: LogicalTick::new(2),
            security: None,
        })),
        "observed_at mismatch must error",
    )?;

    assert!(
        matches!(err, DomainError::DuplicateEvidence { id } if id.value() == 40),
        "expected DuplicateEvidence for evidence, got {:?}",
        err
    );
    Ok(())
}

#[test]
fn record_validation_report_emits_informational_event() -> Result<(), Box<dyn std::error::Error>> {
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
        return Err(std::io::Error::other("expected validation report created event").into());
    }
    assert!(
        output
            .effects
            .iter()
            .any(|effect| matches!(effect, MaestriaEffect::PersistEvent { .. }))
    );
    Ok(())
}
