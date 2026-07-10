use maestria_domain::*;

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
