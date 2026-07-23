use maestria_domain::*;

// ── Task lifecycle and state transitions ──────────────────────────

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
                envelope: Box::new(output.events[0].clone()),
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
        security: None,
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
