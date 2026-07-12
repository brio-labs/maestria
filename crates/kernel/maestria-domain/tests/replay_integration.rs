use maestria_domain::*;

#[test]
fn test_replay_artifact_chunk_card_evidence() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    let art_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(1);
    let card_id = CardId::new(1);
    let claim_id = ClaimId::new(1);
    let ev_id = EvidenceId::new(1);

    // Apply inputs
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: art_id,
        title: "Test Artifact".to_string(),
    }))?;

    state.apply_input(DomainInput::RegisterChunk(RegisterChunkInput {
        chunk_id,
        artifact_id: art_id,
        order: 0,
        text: "chunk text".to_string(),
    }))?;

    state.apply_input(DomainInput::CreateCard(CreateCardInput {
        card_id,
        artifact_id: art_id,
        title: "card title".to_string(),
        body: "card body".to_string(),
    }))?;

    state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id,
        artifact_id: art_id,
        text: "claim text".to_string(),
        evidence_ids: vec![],
    }))?;

    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: ev_id,
        artifact_id: art_id,
        claim_id: Some(claim_id),
        kind: EvidenceKind::FileSpan {
            path: "a".to_string(),
            range: ContentRange { start: 0, end: 1 },
            content_hash: "h".to_string(),
            snapshot: None,
        },
        excerpt: "excerpt text".to_string(),
        observed_at: LogicalTick::new(0),
    }))?;

    // Now check equality of replay
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);
    Ok(())
}

#[test]
fn test_replay_duplicate_rejection() -> Result<(), DomainError> {
    let art_id = ArtifactId::new(1);
    let mut state = KernelState::new();

    let ev = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: art_id,
            title: "Test Artifact".to_string(),
        },
    };

    // First apply works
    state.apply_event(ev.clone())?;

    // Second apply with the exact same event envelope? Wait, ArtifactRegistered doesn't fail on duplicate in apply_event!
    // But ChunkRegistered does.
    let mut ev_chunk = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(1),
            artifact_id: art_id,
            order: 1,
            text: "t".to_string(),
        },
    };
    state.apply_event(ev_chunk.clone())?;

    ev_chunk.id = EventId::new(3);
    ev_chunk.sequence = SequenceNumber::new(3);
    let err = match state.apply_event(ev_chunk) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(
        err,
        DomainError::DuplicateId {
            kind: "chunk",
            id: 1
        }
    ));
    Ok(())
}

#[test]
fn test_task_completion_validation_enforced() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    let rep_id = ValidationReportId::new(1);

    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id,
        title: "T".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Active,
    }))?;

    state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id: rep_id,
            task_id: Some(task_id),
            passed: false, // failed report
            warnings: vec![],
        },
    ))?;

    // Trying to complete with a failed report should fail
    let err = match state.apply_input(DomainInput::CompleteTask(CompleteTaskInput {
        task_id,
        validation_report_id: rep_id,
    })) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::ValidationFailed { task_id }),
    };

    assert!(matches!(err, DomainError::ValidationFailed { .. }));
    Ok(())
}

#[test]
fn replay_accepts_legacy_completion_noop_status_events() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    let report_id = ValidationReportId::new(1);

    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id,
        title: "T".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Active,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Validating,
    }))?;
    state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id,
            task_id: Some(task_id),
            passed: true,
            warnings: vec![],
        },
    ))?;
    state.apply_input(DomainInput::CompleteTask(CompleteTaskInput {
        task_id,
        validation_report_id: report_id,
    }))?;

    let next_event = state.event_log.len() as u64 + 1;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(next_event),
        sequence: SequenceNumber::new(next_event),
        event: DomainEvent::TaskStatusChanged {
            task_id,
            from: TaskStatus::CompletedVerified,
            to: TaskStatus::CompletedVerified,
        },
    })?;
    assert_eq!(
        state.tasks.get(&task_id).map(|task| task.status),
        Some(TaskStatus::CompletedVerified)
    );
    Ok(())
}

#[test]
fn replay_rejects_noncompletion_noop_status_events() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::TaskOpened {
            task_id: TaskId::new(1),
            title: "task".to_string(),
            priority: TaskPriority::Normal,
            artifact_id: None,
        },
    })?;

    let error = state
        .apply_event(DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::Draft,
                to: TaskStatus::Draft,
            },
        })
        .expect_err("noncompletion no-op status events must be rejected");
    assert!(matches!(
        error,
        DomainError::InvalidTaskTransition {
            task_id: TaskId(1),
            from: TaskStatus::Draft,
            to: TaskStatus::Draft,
        }
    ));
    assert_eq!(state.event_log.len(), 1);
    Ok(())
}
#[test]
fn test_out_of_order_sequence_rejection() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    let ev_1 = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(1),
        },
    };
    let ev_2 = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(2),
        },
    };

    state.apply_event(ev_1)?;

    state.apply_event(ev_2)?;

    // Let's test a real failure
    let ev_invalid = DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(5), // expected 3
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(5),
        },
    };

    let err_invalid = match state.apply_event(ev_invalid) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(
        err_invalid,
        DomainError::InvalidSequence {
            expected: 3,
            actual: 5
        }
    ));
    let err_id = match state.apply_event(DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(3),
        },
    }) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(
        err_id,
        DomainError::InvalidEventId {
            expected: 3,
            actual: 4
        }
    ));
    assert_eq!(state.event_log.len(), 2);
    Ok(())
}

#[test]
fn informational_events_validate_referenced_state() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let missing_task = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::UserIntentObserved {
            task_id: TaskId::new(9),
            title: "intent".to_string(),
        },
    };
    assert!(matches!(
        state.apply_event(missing_task),
        Err(DomainError::MissingTask { id }) if id == TaskId::new(9)
    ));

    let missing_artifact = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::SearchCompleted {
            artifact_id: ArtifactId::new(7),
            cards_added: 0,
        },
    };
    assert!(matches!(
        state.apply_event(missing_artifact),
        Err(DomainError::MissingArtifact { id }) if id == ArtifactId::new(7)
    ));
    assert!(state.event_log.is_empty());
    Ok(())
}

#[test]
fn harness_completion_rejects_missing_task() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let err = state
        .apply_input(DomainInput::HarnessRunCompleted(
            maestria_domain::HarnessRunCompleted {
                task_id: Some(TaskId::new(9)),
                command: "test".to_string(),
                exit_code: 1,
                output: String::new(),
            },
        ))
        .expect_err("missing task must reject harness completion");
    assert!(matches!(
        err,
        DomainError::MissingTask { id } if id == TaskId::new(9)
    ));
    assert!(state.event_log.is_empty());
    Ok(())
}

#[test]
fn test_relation_constraints() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let rel_id = RelationId::new(1);

    // Invalid confidence
    let err = match state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: rel_id,
        source: RelationEndpoint::Artifact(ArtifactId::new(99)),
        kind: RelationKind::DerivedFrom,
        target: RelationEndpoint::Artifact(ArtifactId::new(100)),
        evidence_id: None,
        confidence_milli: 1001, // invalid
    })) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(
        err,
        DomainError::InvalidConfidence {
            max: 1000,
            actual: 1001
        }
    ));

    // Missing endpoint
    let err2 = match state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: rel_id,
        source: RelationEndpoint::Artifact(ArtifactId::new(99)), // missing
        kind: RelationKind::DerivedFrom,
        target: RelationEndpoint::Artifact(ArtifactId::new(100)),
        evidence_id: None,
        confidence_milli: 500,
    })) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(err2, DomainError::MissingArtifact { .. }));
    Ok(())
}

#[test]
fn test_claim_evidence_constraints() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    let art_id = ArtifactId::new(1);
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: art_id,
        title: "A".to_string(),
    }))?;

    let ev_id = EvidenceId::new(1);
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: ev_id,
        artifact_id: art_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "a".into(),
            range: ContentRange { start: 1, end: 2 },
            content_hash: "a".into(),
            snapshot: None,
        },
        excerpt: "".to_string(),
        observed_at: LogicalTick::new(1),
    }))?;

    let err = match state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id: ClaimId::new(1),
        artifact_id: art_id,
        text: "T".to_string(),
        evidence_ids: vec![ev_id, ev_id], // duplicate
    })) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };

    assert!(matches!(
        err,
        DomainError::DuplicateId {
            kind: "evidence_in_claim",
            id: 1
        }
    ));

    // Now artifact mismatch
    let art2_id = ArtifactId::new(2);
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: art2_id,
        title: "B".to_string(),
    }))?;

    let err2 = match state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id: ClaimId::new(1),
        artifact_id: art2_id, // mismatch
        text: "T".to_string(),
        evidence_ids: vec![ev_id],
    })) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(
        err2,
        DomainError::ArtifactMismatch {
            expected: _,
            actual: _
        }
    ));
    Ok(())
}

#[test]
fn test_validation_report_constraints() -> Result<(), DomainError> {
    let mut state = KernelState::new();

    // Missing task
    let err = match state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id: ValidationReportId::new(1),
            task_id: Some(TaskId::new(99)), // missing
            passed: true,
            warnings: vec![],
        },
    )) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };
    assert!(matches!(err, DomainError::MissingTask { .. }));
    Ok(())
}

#[test]
fn test_task_completion_status_mismatch() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    let rep_id = ValidationReportId::new(1);

    state.apply_input(DomainInput::OpenTask(OpenTaskInput {
        task_id,
        title: "T".to_string(),
        priority: TaskPriority::Normal,
        artifact_id: None,
    }))?;

    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Open,
    }))?;
    state.apply_input(DomainInput::ChangeTaskStatus(ChangeTaskStatusInput {
        task_id,
        to: TaskStatus::Active,
    }))?;

    // Report with warnings
    state.apply_input(DomainInput::RecordValidationReport(
        RecordValidationReportInput {
            report_id: rep_id,
            task_id: Some(task_id),
            passed: true,
            warnings: vec!["warning".to_string()],
        },
    ))?;

    // Can't complete verified if there are warnings!
    // Let's craft an envelope directly because apply_input doesn't allow bypassing the helper's automatic status
    let ev_invalid_status = DomainEventEnvelope {
        id: EventId::new(5),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::TaskCompletionRecorded {
            task_id,
            status: TaskStatus::CompletedVerified,
            validation_report_id: rep_id,
        },
    };

    let err = match state.apply_event(ev_invalid_status) {
        Err(e) => e,
        Ok(_) => return Err(DomainError::EmptyIntent),
    };

    assert!(matches!(
        err,
        DomainError::ValidationWarningsForbidden { .. }
    ));
    Ok(())
}

#[test]
fn replay_ingestion_flow_state_parity() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "content".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);
    Ok(())
}

#[test]
fn replay_ingestion_flow_with_multiple_chunks() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Big Doc".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                order: 0,
                text: "chunk a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                order: 1,
                text: "chunk b".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(12),
                artifact_id: ArtifactId::new(1),
                order: 2,
                text: "chunk c".to_string(),
            },
        ],
        cards: vec![
            CreateCardInput {
                card_id: CardId::new(20),
                artifact_id: ArtifactId::new(1),
                title: "Card 1".to_string(),
                body: "Alpha".to_string(),
            },
            CreateCardInput {
                card_id: CardId::new(21),
                artifact_id: ArtifactId::new(1),
                title: "Card 2".to_string(),
                body: "Beta".to_string(),
            },
        ],
    }))?;

    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);
    assert_eq!(replayed.chunks.len(), 3);
    assert_eq!(replayed.cards.len(), 2);
    assert_eq!(
        replayed
            .artifacts
            .get(&ArtifactId::new(1))
            .ok_or(DomainError::MissingArtifact {
                id: ArtifactId::new(1),
            })?
            .chunk_ids
            .len(),
        3
    );
    Ok(())
}

#[test]
fn replay_ingestion_detection_only() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Pending".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);
    assert!(replayed.chunks.is_empty());
    assert!(replayed.cards.is_empty());
    assert_eq!(replayed.event_log.len(), 2);
    Ok(())
}
#[test]
fn replay_ingestion_duplicate_chunk_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "unique".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // event_log now has: ArtifactRegistered (id=1), PendingIndex (id=2), ChunkRegistered (id=3), ArtifactParsed (id=4)
    let next_id = state.event_log.len() as u64 + 1;
    let duplicate_chunk = DomainEventEnvelope {
        id: EventId::new(next_id),
        sequence: SequenceNumber::new(next_id),
        event: DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "duplicate".to_string(),
        },
    };
    let err = state
        .apply_event(duplicate_chunk)
        .expect_err("duplicate chunk in replay must error");
    assert!(matches!(
        err,
        DomainError::DuplicateId {
            kind: "chunk",
            id: 10,
        }
    ));
    Ok(())
}

#[test]
fn replay_ingestion_parser_without_detection_rejected() -> Result<(), DomainError> {
    let next_id = 1u64;
    let orphan_artparsed = DomainEventEnvelope {
        id: EventId::new(next_id),
        sequence: SequenceNumber::new(next_id),
        event: DomainEvent::ArtifactParsed {
            artifact_id: ArtifactId::new(99),
            chunks_added: 0,
        },
    };
    let mut state = KernelState::new();
    let err = state
        .apply_event(orphan_artparsed)
        .expect_err("ArtifactParsed without artifact must error");
    assert!(matches!(
        err,
        DomainError::MissingArtifact { id } if id == ArtifactId::new(99)
    ));
    assert!(state.event_log.is_empty());
    Ok(())
}

#[test]
fn replay_ingestion_orphan_chunk_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let orphan_chunk = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(1),
            artifact_id: ArtifactId::new(99),
            order: 0,
            text: "orphan".to_string(),
        },
    };
    let err = state
        .apply_event(orphan_chunk)
        .expect_err("ChunkRegistered without artifact must error");
    assert!(matches!(
        err,
        DomainError::MissingArtifact { id } if id == ArtifactId::new(99)
    ));
    Ok(())
}
