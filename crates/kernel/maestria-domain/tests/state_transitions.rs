use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

// ── Task lifecycle and state transitions ──────────────────────────

#[test]
fn parser_completed_registers_chunks_and_cards() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
    }))?;

    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "first chunk".to_string(),
        }],
        cards: vec![CreateCardInput {
            card_id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Parsed summary".to_string(),
        }],
    }))?;

    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    assert!(state.cards.contains_key(&CardId::new(20)));
    assert!(
        state
            .artifacts
            .get(&ArtifactId::new(1))
            .is_some_and(|artifact| artifact.chunk_ids.contains(&ChunkId::new(10))
                && artifact.card_ids.contains(&CardId::new(20)))
    );
    assert!(output.events.iter().any(|event| matches!(
        event.event,
        DomainEvent::CardCreated {
            card_id: CardId(20),
            artifact_id: ArtifactId(1),
            ..
        }
    )));
    Ok(())
}

#[test]
fn task_status_transition_is_restricted() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(3),
        title: "initial".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;

    assert!(
        state
            .apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
                task_id: TaskId::new(3),
                to: TaskStatus::Active,
            }))
            .is_err()
    );

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(3),
        to: TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(3),
        to: TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(3),
        to: TaskStatus::Validating,
    }))?;
    assert!(matches!(
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id: TaskId::new(3),
            to: TaskStatus::CompletedVerified,
        })),
        Err(DomainError::ValidationRequired { .. })
    ));
    state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id: ValidationReportId::new(9),
            task_id: Some(TaskId::new(3)),
            passed: true,
            warnings: Vec::new(),
        },
    ))?;
    state.apply_input(DomainInput::CompleteTask(CompleteTaskInput {
        task_id: TaskId::new(3),
        validation_report_id: ValidationReportId::new(9),
    }))?;

    let task = state
        .tasks
        .get(&TaskId::new(3))
        .ok_or(DomainError::MissingTask { id: TaskId::new(3) })?;
    assert_eq!(task.status, TaskStatus::CompletedVerified);
    assert_eq!(task.validation_report_id, Some(ValidationReportId::new(9)));
    Ok(())
}

#[test]
fn validated_completion_is_the_only_completion_path() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(7),
        title: "Ship the verified answer".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(7),
        to: TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(7),
        to: TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id: ValidationReportId::new(80),
            task_id: Some(TaskId::new(7)),
            passed: true,
            warnings: vec!["non-blocking warning".to_string()],
        },
    ))?;

    assert_eq!(
        state
            .apply_input(DomainInput::CompleteTask(CompleteTaskInput {
                task_id: TaskId::new(7),
                validation_report_id: ValidationReportId::new(80),
            }))
            .err(),
        Some(DomainError::InvalidTaskTransition {
            task_id: TaskId::new(7),
            from: TaskStatus::Active,
            to: TaskStatus::CompletedWithWarnings,
        })
    );

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(7),
        to: TaskStatus::Validating,
    }))?;
    let output = state.apply_input(DomainInput::CompleteTask(CompleteTaskInput {
        task_id: TaskId::new(7),
        validation_report_id: ValidationReportId::new(80),
    }))?;

    let task = state
        .tasks
        .get(&TaskId::new(7))
        .ok_or(DomainError::MissingTask { id: TaskId::new(7) })?;
    assert_eq!(task.status, TaskStatus::CompletedWithWarnings);
    assert_eq!(task.validation_report_id, Some(ValidationReportId::new(80)));
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            },
            ..
        }] if *task_id == TaskId::new(7)
            && *status == TaskStatus::CompletedWithWarnings
            && *validation_report_id == ValidationReportId::new(80)
    ));
    assert_eq!(
        output.effects,
        vec![
            MaestriaEffect::PersistEvent {
                envelope: output.events[0].clone(),
            },
            MaestriaEffect::PersistState(PersistStateRequest {
                reason: "validated task completion".to_string(),
            }),
        ]
    );
    Ok(())
}

#[test]
fn complete_task_requires_validation_report() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(7),
        title: "Ship the verified answer".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(7),
        to: TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(7),
        to: TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id: TaskId::new(7),
        to: TaskStatus::Validating,
    }))?;

    assert_eq!(
        state
            .apply_input(DomainInput::CompleteTask(CompleteTaskInput {
                task_id: TaskId::new(7),
                validation_report_id: ValidationReportId::new(80),
            }))
            .err(),
        Some(DomainError::MissingValidationReport {
            id: ValidationReportId::new(80)
        })
    );
    Ok(())
}

#[test]
fn test_all_legal_task_transitions() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    let art_id = ArtifactId::new(1);

    // Register artifact so we have a target
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: art_id,
        title: "Test".to_string(),
    }))?;

    // Open task
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id,
        title: "T".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: Some(art_id),
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Open,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Active,
    }))?;

    // Transition to Blocked
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Blocked,
    }))?;

    // Back to Active
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Active,
    }))?;

    // We can't transition directly to CompletedVerified via ChangeTaskStatus due to ValidationRequired
    let err = match state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::CompletedVerified,
    })) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::ValidationRequired { task_id }),
    };
    assert!(matches!(err, DomainError::ValidationRequired { .. }));

    // Replay property: sequential transitions must be fully isomorphic to event stream application
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);

    Ok(())
}
#[test]
fn validating_transition_emits_run_validation_effect() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let task_id = TaskId::new(42);

    // Open task
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id,
        title: "Test validation".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;

    // Transition to Open
    let out_open = state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Open,
    }))?;

    // Assert NO validation effect for non-Validating transition
    assert!(
        !out_open
            .effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::RunValidation(_))),
        "Should not emit RunValidation effect on Open transition"
    );

    // Transition to Active
    let out_active = state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Active,
    }))?;

    assert!(
        !out_active
            .effects
            .iter()
            .any(|e| matches!(e, MaestriaEffect::RunValidation(_))),
        "Should not emit RunValidation effect on Active transition"
    );

    // Transition to Validating
    let out_validating =
        state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
            task_id,
            to: TaskStatus::Validating,
        }))?;

    assert_eq!(
        out_validating.effects.len(),
        2,
        "Should emit exactly two effects: PersistEvent and RunValidation"
    );

    assert!(
        matches!(
            out_validating.effects[0],
            MaestriaEffect::PersistEvent { .. }
        ),
        "First effect must be PersistEvent"
    );

    let req = match &out_validating.effects[1] {
        MaestriaEffect::RunValidation(req) => req,
        _ => {
            return Err(DomainError::InternalInvariantViolation {
                detail: "Second effect must be RunValidation",
            });
        }
    };

    assert_eq!(req.task_id, Some(task_id));
    assert_eq!(req.claim_id, None);
    assert_eq!(req.validation_report_id, ValidationReportId::new(0));
    Ok(())
}

// ── Task evidence linking ──────────────────────────────────────────

#[test]
fn link_evidence_to_task_succeeds() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(10),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes.txt".to_string(),
            range: ContentRange { start: 1, end: 2 },
            content_hash: "sha256:notes".to_string(),
            snapshot: None,
        },
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(1),
    }))?;
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(3),
        title: "Review notes".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: Some(ArtifactId::new(1)),
    }))?;

    let output = state.apply_input(DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
        task_id: TaskId::new(3),
        evidence_id: EvidenceId::new(10),
    }))?;

    let task = state
        .tasks
        .get(&TaskId::new(3))
        .ok_or(DomainError::MissingTask { id: TaskId::new(3) })?;
    assert!(task.evidence_ids.contains(&EvidenceId::new(10)));

    assert!(
        output
            .events
            .iter()
            .any(|e| matches!(e.event, DomainEvent::TaskEvidenceLinked { .. }))
    );

    // Replay is deterministic
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);

    Ok(())
}

// ── SearchExecuted audit event ────────────────────────────────────

#[test]
fn search_executed_emits_audit_event_with_evidence_ids() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "hello world".to_string(),
        limit: 10,
        evidence_ids: vec![EvidenceId::new(1), EvidenceId::new(2)],
        at: LogicalTick::new(42),
    }))?;

    assert_eq!(output.events.len(), 1);
    assert_eq!(output.effects.len(), 1);
    let envelope = &output.events[0];
    match &envelope.event {
        DomainEvent::SearchExecuted {
            query,
            limit,
            evidence_ids,
            at,
        } => {
            assert_eq!(query, "hello world");
            assert_eq!(*limit, 10);
            assert_eq!(evidence_ids, &vec![EvidenceId::new(1), EvidenceId::new(2)]);
            assert_eq!(*at, LogicalTick::new(42));
        }
        _ => {
            return Err(DomainError::InternalInvariantViolation {
                detail: "expected SearchExecuted event",
            });
        }
    }
    // Audit events must not mutate any entity collections.
    assert!(state.artifacts.is_empty());
    assert!(state.cards.is_empty());
    assert!(state.evidences.is_empty());
    assert_eq!(state.event_log.len(), 1);
    Ok(())
}

#[test]
fn link_evidence_to_task_idempotent() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(10),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes.txt".to_string(),
            range: ContentRange { start: 1, end: 2 },
            content_hash: "sha256:notes".to_string(),
            snapshot: None,
        },
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(1),
    }))?;
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(3),
        title: "Review notes".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: Some(ArtifactId::new(1)),
    }))?;

    // First link
    state.apply_input(DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
        task_id: TaskId::new(3),
        evidence_id: EvidenceId::new(10),
    }))?;

    // Second link (duplicate) — should be idempotent
    let output = state.apply_input(DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
        task_id: TaskId::new(3),
        evidence_id: EvidenceId::new(10),
    }))?;

    let task = state
        .tasks
        .get(&TaskId::new(3))
        .ok_or(DomainError::MissingTask { id: TaskId::new(3) })?;
    assert_eq!(task.evidence_ids.len(), 1);
    assert!(task.evidence_ids.contains(&EvidenceId::new(10)));

    // Duplicate links do not emit another event or persistence effect.

    assert!(output.events.is_empty());
    assert!(output.effects.is_empty());
    Ok(())
}

#[test]
fn search_executed_rejects_empty_query() {
    let mut state = KernelState::new();
    let err = state
        .apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
            query: "   ".to_string(),
            limit: 5,
            evidence_ids: vec![],
            at: LogicalTick::new(1),
        }))
        .expect_err("empty query must be rejected");
    assert!(matches!(err, DomainError::EmptyIntent));
}

#[test]
fn search_executed_is_deterministic_on_replay() -> Result<(), DomainError> {
    let mut state_a = KernelState::new();
    state_a.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "deterministic".to_string(),
        limit: 3,
        evidence_ids: vec![EvidenceId::new(10)],
        at: LogicalTick::new(7),
    }))?;

    let replayed = replay_events(&state_a.event_log)?;
    assert_eq!(state_a, replayed);
    Ok(())
}

#[test]
fn link_evidence_to_missing_task_is_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
    }))?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(10),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes.txt".to_string(),
            range: ContentRange { start: 1, end: 2 },
            content_hash: "sha256:notes".to_string(),
            snapshot: None,
        },
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(1),
    }))?;

    let result = state.apply_input(DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
        task_id: TaskId::new(99),
        evidence_id: EvidenceId::new(10),
    }));

    assert!(matches!(result, Err(DomainError::MissingTask { .. })));
    Ok(())
}

#[test]
fn link_evidence_to_task_missing_evidence_is_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id: TaskId::new(3),
        title: "Review notes".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;

    let result = state.apply_input(DomainInput::LinkEvidenceToTask(LinkEvidenceToTaskInput {
        task_id: TaskId::new(3),
        evidence_id: EvidenceId::new(99),
    }));

    assert!(matches!(result, Err(DomainError::MissingEvidence { .. })));
    Ok(())
}

#[test]
fn search_executed_persist_effect_matches_event_envelope() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::SearchExecuted(SearchExecutedInput {
        query: "audit".to_string(),
        limit: 1,
        evidence_ids: vec![],
        at: LogicalTick::new(1),
    }))?;

    let envelope = match output.effects.as_slice() {
        [MaestriaEffect::PersistEvent { envelope }] => envelope,
        _ => return Err(DomainError::EmptyIntent),
    };
    assert_eq!(envelope, &output.events[0]);
    Ok(())
}
