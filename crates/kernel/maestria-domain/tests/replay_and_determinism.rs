use maestria_domain::*;
#[path = "common/replay.rs"]
mod common;
use common::{file_span_kind, run_replay_once};

// ── Replay determinism and event reconstruction ───────────────────

#[test]
fn replay_events_reconstructs_new_memory_event_state() -> Result<(), DomainError> {
    let inputs = vec![
        DomainInput::RegisterArtifact(RegisterArtifactInput {
            artifact_id: ArtifactId::new(1),
            title: "Project Notes".to_string(),
        }),
        DomainInput::CreateClaim(CreateClaimInput {
            claim_id: ClaimId::new(20),
            artifact_id: ArtifactId::new(1),
            text: "Claim from evidence".to_string(),
            evidence_ids: Vec::new(),
        }),
        DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: file_span_kind(),
            excerpt: "first chunk".to_string(),
            observed_at: LogicalTick::new(12),
        }),
        DomainInput::CreateMemoryCandidate(CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(90),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 720,
        }),
        DomainInput::CreateMemoryCandidate(CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(91),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 650,
        }),
        DomainInput::PromoteMemory(PromoteMemoryInput {
            memory_id: MemoryId::new(100),
            candidate_id: MemoryCandidateId::new(90),
        }),
        DomainInput::PromoteMemory(PromoteMemoryInput {
            memory_id: MemoryId::new(101),
            candidate_id: MemoryCandidateId::new(91),
        }),
        DomainInput::ContradictMemory(ContradictMemoryInput {
            memory_id: MemoryId::new(100),
            contradicting_candidate_id: MemoryCandidateId::new(91),
        }),
        DomainInput::DeprecateMemory(DeprecateMemoryInput {
            memory_id: MemoryId::new(101),
        }),
        DomainInput::SupersedeMemory(SupersedeMemoryInput {
            memory_id: MemoryId::new(100),
            by_memory_id: MemoryId::new(101),
        }),
        DomainInput::RecordValidationReport(RecordValidationReportInput {
            report_id: ValidationReportId::new(80),
            task_id: None,
            passed: false,
            warnings: Vec::new(),
        }),
    ];
    let (_state, events, _effects) = replay_inputs(&inputs)?;
    let replayed = replay_events(&events)?;

    assert_eq!(replayed.event_log, events);
    assert_eq!(
        replayed
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Superseded
    );
    assert_eq!(
        replayed
            .memories
            .get(&MemoryId::new(101))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(101),
            })?
            .status,
        MemoryStatus::Deprecated
    );
    assert!(events.iter().any(|event| matches!(
        event.event,
        DomainEvent::ValidationReportCreated {
            report_id: ValidationReportId(80),
            task_id: None,
            passed: false,
            ..
        }
    )));
    Ok(())
}

fn deterministic_shape_inputs() -> Vec<DomainInput> {
    vec![
        DomainInput::RegisterArtifact(RegisterArtifactInput {
            artifact_id: ArtifactId::new(1),
            title: "Project Notes".to_string(),
        }),
        DomainInput::CreateClaim(CreateClaimInput {
            claim_id: ClaimId::new(20),
            artifact_id: ArtifactId::new(1),
            text: "Claim from evidence".to_string(),
            evidence_ids: Vec::new(),
        }),
        DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: file_span_kind(),
            excerpt: "first chunk".to_string(),
            observed_at: LogicalTick::new(12),
        }),
        DomainInput::OpenTask(OpenTaskInput {
            task_id: TaskId::new(50),
            title: "Summarize artifact".to_string(),
            priority: TaskPriority::Normal,
            artifact_id: Some(ArtifactId::new(1)),
        }),
        DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(50),
            to: TaskStatus::Open,
        }),
        DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(50),
            to: TaskStatus::Active,
        }),
        DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(50),
            to: TaskStatus::Validating,
        }),
        DomainInput::RecordValidationReport(RecordValidationReportInput {
            report_id: ValidationReportId::new(80),
            task_id: Some(TaskId::new(50)),
            passed: true,
            warnings: Vec::new(),
        }),
        DomainInput::CompleteTask(CompleteTaskInput {
            task_id: TaskId::new(50),
            validation_report_id: ValidationReportId::new(80),
        }),
        DomainInput::CreateRelation(CreateRelationInput {
            relation_id: RelationId::new(70),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Task(TaskId::new(50)),
            evidence_id: Some(EvidenceId::new(40)),
            confidence_milli: 875,
        }),
        DomainInput::CreateMemoryCandidate(CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(90),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 720,
        }),
    ]
}
#[test]
fn replay_keeps_new_event_and_effect_shapes_deterministic() -> Result<(), DomainError> {
    let inputs = deterministic_shape_inputs();

    let (state_a, events_a, effects_a) = replay_inputs(&inputs)?;
    let (state_b, events_b, effects_b) = replay_inputs(&inputs)?;

    assert_eq!(state_a, state_b);
    assert_eq!(events_a, events_b);
    assert_eq!(effects_a, effects_b);
    assert!(events_a.iter().any(|envelope| matches!(
        &envelope.event,
        DomainEvent::TaskCompletionRecorded {
            task_id,
            status,
            validation_report_id,
        } if *task_id == TaskId::new(50)
            && *status == TaskStatus::CompletedVerified
            && *validation_report_id == ValidationReportId::new(80)
    )));
    assert!(events_a.iter().any(|envelope| matches!(
        &envelope.event,
        DomainEvent::RelationCreated { relation_id, .. } if *relation_id == RelationId::new(70)
    )));
    assert!(events_a.iter().any(|envelope| matches!(
        &envelope.event,
        DomainEvent::MemoryCandidateCreated {
            candidate_id,
            claim_id,
            ..
        } if *candidate_id == MemoryCandidateId::new(90)
            && *claim_id == ClaimId::new(20)
    )));
    assert!(effects_a.iter().any(|effect| matches!(
        effect,
        MaestriaEffect::PersistState(PersistStateRequest { reason })
            if reason == "validated task completion"
    )));
    assert!(effects_a.iter().any(|effect| matches!(
        effect,
        MaestriaEffect::UpdateGraph(UpdateGraphRequest { relation_id })
            if *relation_id == RelationId::new(70)
    )));
    Ok(())
}

#[test]
fn persist_effects_keep_exact_event_envelopes() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let first = state.apply_input(DomainInput::ClockTick(LogicalTick::new(7)))?;
    let second = state.apply_input(DomainInput::ClockTick(LogicalTick::new(7)))?;
    let first_envelope = match first.effects.as_slice() {
        [MaestriaEffect::PersistEvent { envelope }] => envelope,
        _ => return Err(DomainError::EmptyIntent),
    };
    let second_envelope = match second.effects.as_slice() {
        [MaestriaEffect::PersistEvent { envelope }] => envelope,
        _ => return Err(DomainError::EmptyIntent),
    };
    assert_eq!(first_envelope, &first.events[0]);
    assert_eq!(second_envelope, &second.events[0]);
    assert_ne!(first_envelope.id, second_envelope.id);
    assert_ne!(first_envelope.sequence, second_envelope.sequence);
    Ok(())
}

#[test]
fn replay_is_deterministic() -> Result<(), DomainError> {
    let (state_a, events_a, effects_a) = run_replay_once()?;
    let (state_b, events_b, effects_b) = run_replay_once()?;

    assert_eq!(state_a, state_b);
    assert_eq!(events_a, events_b);
    assert_eq!(effects_a, effects_b);
    Ok(())
}

#[test]
fn replay_events_are_equivalent() -> Result<(), DomainError> {
    let (state, events, _) = run_replay_once()?;
    let replayed = replay_events(&events)?;
    assert_eq!(state.event_log, replayed.event_log);
    assert_eq!(state.artifacts.len(), replayed.artifacts.len());
    assert_eq!(state.claims.len(), replayed.claims.len());
    Ok(())
}
