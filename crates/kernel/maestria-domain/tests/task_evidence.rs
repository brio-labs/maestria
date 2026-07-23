use maestria_domain::*;

// ── Task evidence linking ─────────────────────────────────────────

#[test]
fn link_evidence_to_task_succeeds() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
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
        security: None,
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
fn link_evidence_to_task_idempotent() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
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
        security: None,
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
fn link_evidence_to_missing_task_is_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        security: None,
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
        security: None,
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
