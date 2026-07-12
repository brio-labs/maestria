use std::collections::BTreeSet;

use maestria_domain::*;
fn sample_inputs() -> Vec<DomainInput> {
    vec![
        DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "Project Notes".to_string(),
            source_path: "notes.txt".to_string(),
            source_bytes: b"project notes content".to_vec(),
            content_hash: "sha256:abc".to_string(),
        }),
        DomainInput::ParserCompleted(ParserResult {
            artifact_id: ArtifactId::new(1),
            chunks: vec![
                RegisterChunkInput {
                    chunk_id: ChunkId::new(10),
                    artifact_id: ArtifactId::new(1),
                    order: 0,
                    text: "first chunk".to_string(),
                },
                RegisterChunkInput {
                    chunk_id: ChunkId::new(11),
                    artifact_id: ArtifactId::new(1),
                    order: 1,
                    text: "second chunk".to_string(),
                },
            ],
            cards: Vec::new(),
        }),
        DomainInput::CreateClaim(CreateClaimInput {
            claim_id: ClaimId::new(20),
            artifact_id: ArtifactId::new(1),
            text: "Claim from evidence".to_string(),
            evidence_ids: Vec::new(),
        }),
        DomainInput::CreateCard(CreateCardInput {
            card_id: CardId::new(30),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Summarize project notes".to_string(),
        }),
        DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(20)),
            kind: EvidenceKind::FileSpan {
                path: "notes.txt".to_string(),
                range: ContentRange { start: 1, end: 2 },
                content_hash: "sha256:notes".to_string(),
                snapshot: None,
            },
            excerpt: "first chunk".to_string(),
            observed_at: LogicalTick::new(12),
        }),
        DomainInput::LinkEvidenceToClaim(LinkEvidenceToClaimInput {
            claim_id: ClaimId::new(20),
            evidence_id: EvidenceId::new(40),
        }),
        DomainInput::UserIntent(UserIntent {
            task_id: TaskId::new(50),
            title: "Summarize artifact".to_string(),
            priority: TaskPriority::Normal,
        }),
        DomainInput::ValidationCompleted(ValidationCompleted {
            claim_id: ClaimId::new(20),
            valid: true,
        }),
        DomainInput::ClockTick(LogicalTick::new(99)),
    ]
}

fn run_replay_once()
-> Result<(KernelState, Vec<DomainEventEnvelope>, Vec<MaestriaEffect>), DomainError> {
    replay_inputs(&sample_inputs())
}

fn register_artifact_and_claim(state: &mut KernelState) -> Result<(), DomainError> {
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
    }))?;
    state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id: ClaimId::new(20),
        artifact_id: ArtifactId::new(1),
        text: "Claim from evidence".to_string(),
        evidence_ids: Vec::new(),
    }))?;
    Ok(())
}

fn file_span_kind() -> EvidenceKind {
    EvidenceKind::FileSpan {
        path: "notes.txt".to_string(),
        range: ContentRange { start: 1, end: 2 },
        content_hash: "sha256:notes".to_string(),
        snapshot: None,
    }
}

fn state_with_memory_candidate(
    candidate_id: MemoryCandidateId,
) -> Result<KernelState, DomainError> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_span_kind(),
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(12),
    }))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id,
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 720,
        },
    ))?;
    Ok(state)
}

fn promote_memory(
    state: &mut KernelState,
    memory_id: MemoryId,
    candidate_id: MemoryCandidateId,
) -> Result<(), DomainError> {
    state.apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
        memory_id,
        candidate_id,
    }))?;
    Ok(())
}

#[test]
fn parser_completed_registers_chunks_and_cards() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
    }))?;

    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
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

#[test]
fn relation_and_memory_candidates_are_domain_owned_and_evidence_bound() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
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

    let relation_output = state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: RelationId::new(70),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: Some(EvidenceId::new(40)),
        confidence_milli: 875,
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

    assert!(matches!(
        state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(91),
                claim_id: ClaimId::new(20),
                evidence_ids: Vec::new(),
                confidence_milli: 720,
            },
        )),
        Err(DomainError::EvidenceRequired {
            kind: "memory_candidate",
            id: 91,
        })
    ));

    let candidate_output = state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(90),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40), EvidenceId::new(40)],
            confidence_milli: 720,
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

#[test]
fn replay_keeps_new_event_and_effect_shapes_deterministic() -> Result<(), DomainError> {
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
    ];

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

#[test]
fn preflight_artifact_detected_creates_artifact() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;

    assert!(state.artifacts.contains_key(&ArtifactId::new(1)));
    assert_eq!(state.artifacts[&ArtifactId::new(1)].title, "Notes");
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].content_hash,
        Some("sha256:abc".to_string())
    );
    assert_eq!(output.events.len(), 2);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId(1),
            ..
        }
    ));
    assert!(matches!(
        &output.events[1].event,
        DomainEvent::PendingIndex {
            artifact_id: ArtifactId(1),
            ..
        }
    ));
    Ok(())
}

#[test]
fn preflight_duplicate_is_noop_when_indexed() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Set up artifact as fully indexed via replay events
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::PendingIndex {
            artifact_id: ArtifactId::new(1),
            content_hash: "sha256:abc".to_string(),
        },
    })?;
    state.apply_event(DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId::new(1),
        },
    })?;

    // Re-detection with same hash while Indexed is a no-op
    let output = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;

    assert_eq!(
        output.events.len(),
        0,
        "no events for unchanged indexed artifact"
    );
    assert_eq!(
        output.effects.len(),
        0,
        "no effects for unchanged indexed artifact"
    );
    Ok(())
}

#[test]
fn detection_without_parser_leaves_no_chunks_or_cards() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    assert!(state.artifacts.contains_key(&ArtifactId::new(1)));
    assert!(state.chunks.is_empty(), "no chunks before parsing");
    assert!(state.cards.is_empty(), "no cards before parsing");
    let artifact = &state.artifacts[&ArtifactId::new(1)];
    assert!(
        artifact.chunk_ids.is_empty(),
        "artifact references no chunks before parsing"
    );
    Ok(())
}

#[test]
fn parser_without_prior_detection_is_rejected() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    let err = state
        .apply_input(DomainInput::ParserCompleted(ParserResult {
            artifact_id: ArtifactId::new(1),
            chunks: vec![RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                order: 0,
                text: "lonely chunk".to_string(),
            }],
            cards: Vec::new(),
        }))
        .expect_err("parser without detection must error");
    assert!(matches!(err, DomainError::MissingArtifact { id } if id == ArtifactId::new(1)));
    Ok(())
}

#[test]
fn full_ingestion_flow_detection_then_parsing() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Preflight
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    }))?;
    let output = state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                order: 0,
                text: "first chunk".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                order: 1,
                text: "second chunk".to_string(),
            },
        ],
        cards: vec![CreateCardInput {
            card_id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "Summary".to_string(),
            body: "Document summary".to_string(),
        }],
    }))?;

    // Artifact references chunks and cards
    let artifact = &state.artifacts[&ArtifactId::new(1)];
    assert_eq!(artifact.chunk_ids.len(), 2);
    assert_eq!(artifact.card_ids.len(), 1);
    assert!(state.chunks.contains_key(&ChunkId::new(10)));
    assert!(state.chunks.contains_key(&ChunkId::new(11)));
    assert!(state.cards.contains_key(&CardId::new(20)));

    // Events include: 2 chunks, 1 card, 1 ArtifactParsed
    let chunk_reg_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ChunkRegistered { .. }))
        .count();
    let card_created_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::CardCreated { .. }))
        .count();
    let parsed_count = output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count();

    assert_eq!(chunk_reg_count, 2, "two chunk events");
    assert_eq!(card_created_count, 1, "one card event");
    assert_eq!(parsed_count, 1, "one artifact-parsed terminal event");

    Ok(())
}

#[test]
fn ingestion_replay_from_detection_only() -> Result<(), DomainError> {
    let inputs = vec![DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: Vec::new(),
        content_hash: "sha256:abc".to_string(),
    })];
    let (state, events, _effects) = replay_inputs(&inputs)?;
    assert!(state.artifacts.contains_key(&ArtifactId::new(1)));
    assert!(state.chunks.is_empty());
    assert_eq!(events.len(), 2);
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );
    // Replay the events into a fresh state
    let replayed = replay_events(&events)?;
    assert_eq!(state, replayed);
    Ok(())
}

#[test]
fn ingestion_replay_full_flow_reconstructs_state() -> Result<(), DomainError> {
    let inputs = vec![
        DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
            source_path: String::new(),
            source_bytes: Vec::new(),
            content_hash: "sha256:abc".to_string(),
        }),
        DomainInput::ParserCompleted(ParserResult {
            artifact_id: ArtifactId::new(1),
            chunks: vec![
                RegisterChunkInput {
                    chunk_id: ChunkId::new(10),
                    artifact_id: ArtifactId::new(1),
                    order: 0,
                    text: "chunk one".to_string(),
                },
                RegisterChunkInput {
                    chunk_id: ChunkId::new(11),
                    artifact_id: ArtifactId::new(1),
                    order: 1,
                    text: "chunk two".to_string(),
                },
            ],
            cards: vec![CreateCardInput {
                card_id: CardId::new(20),
                artifact_id: ArtifactId::new(1),
                title: "Summary".to_string(),
                body: "Parsed doc".to_string(),
            }],
        }),
    ];
    let (state, events, _effects) = replay_inputs(&inputs)?;

    assert_eq!(state.artifacts.len(), 1);
    assert_eq!(state.chunks.len(), 2);
    assert_eq!(state.cards.len(), 1);

    // Replay from events produces identical state
    let replayed = replay_events(&events)?;
    assert_eq!(state, replayed);
    Ok(())
}

#[test]
fn ingestion_replay_rejects_duplicate_detection_events() -> Result<(), DomainError> {
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
        },
    };
    let mut state = KernelState::new();
    state.apply_event(event.clone())?;

    let duplicate = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "Notes".to_string(),
        },
    };
    let err = state
        .apply_event(duplicate)
        .expect_err("duplicate artifact registration in replay must error");
    assert!(matches!(
        err,
        DomainError::DuplicateId {
            kind: "artifact",
            id: 1,
        }
    ));
    Ok(())
}

#[test]
fn changed_hash_emits_new_pending_index() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // First detection
    let output1 = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:aaa".to_string(),
    }))?;
    assert_eq!(output1.events.len(), 2);

    // Second detection with different hash
    let output2 = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![4, 5, 6],
        content_hash: "sha256:bbb".to_string(),
    }))?;

    // Should emit PendingIndex again (but not ArtifactRegistered since artifact exists)
    assert_eq!(output2.events.len(), 1);
    assert!(matches!(
        &output2.events[0].event,
        DomainEvent::PendingIndex { content_hash, .. } if content_hash == "sha256:bbb"
    ));
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].content_hash,
        Some("sha256:bbb".to_string())
    );
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );
    Ok(())
}

#[test]
fn pending_detection_not_treated_as_unchanged() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // First detection → Pending
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );

    // Second detection with same hash while Pending — must not be no-op
    let output = state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;

    // Still emits PendingIndex + effects (re-drives the pipeline)
    assert_eq!(output.events.len(), 1);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::PendingIndex { .. }
    ));
    assert!(
        !output.effects.is_empty(),
        "should re-emit store/parse effects"
    );
    Ok(())
}

#[test]
fn full_text_index_partial_feedback() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Detect
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    // Parse with two chunks
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                order: 0,
                text: "a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                order: 1,
                text: "b".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;
    // Both chunks are pending
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );

    // Full-text index completes for one chunk
    let output = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;

    // Chunk 10 removed from pending
    assert!(!state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));
    // Still Pending — not all chunks indexed
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Pending
    );
    // Emitted FullTextIndexed but not ArtifactIndexed
    assert_eq!(output.events.len(), 1);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::FullTextIndexed {
            chunk_id: ChunkId(10),
            ..
        }
    ));
    Ok(())
}

#[test]
fn full_text_index_final_feedback_emits_artifact_indexed() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Detect + Parse with one chunk
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "a".to_string(),
        }],
        cards: Vec::new(),
    }))?;
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));

    // Full-text index completes for the only chunk
    let output = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;

    // All chunks indexed → ArtifactIndexed
    assert!(state.pending_full_text.is_empty());
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert_eq!(output.events.len(), 2);
    assert!(matches!(
        &output.events[0].event,
        DomainEvent::FullTextIndexed { .. }
    ));
    assert!(matches!(
        &output.events[1].event,
        DomainEvent::ArtifactIndexed {
            artifact_id: ArtifactId(1)
        }
    ));
    Ok(())
}

#[test]
fn duplicate_full_text_index_feedback_is_idempotent() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Detect + Parse
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 0,
            text: "a".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    // First feedback
    let output1 = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    assert_eq!(output1.events.len(), 2); // FullTextIndexed + ArtifactIndexed

    // Second feedback for same chunk — idempotent
    let output2 = state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    assert_eq!(
        output2.events.len(),
        0,
        "duplicate feedback must be idempotent"
    );
    assert_eq!(output2.effects.len(), 0);
    Ok(())
}

#[test]
fn replay_reconstructs_pending_and_indexed_state() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    // Full flow: detect → parse → index all chunks
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Notes".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                order: 0,
                text: "a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                order: 1,
                text: "b".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;
    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(11),
        },
    ))?;

    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert!(state.pending_full_text.is_empty());

    // Replay events reconstructs identical state
    let replayed = replay_events(&state.event_log)?;
    assert_eq!(state, replayed);
    assert_eq!(
        replayed.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Indexed
    );
    assert!(replayed.pending_full_text.is_empty());
    Ok(())
}

#[test]
fn kernel_does_not_depend_on_forbidden_runtime_crates_or_operators() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"));
    let prelude = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(head, _)| head);
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));

    for forbidden in ["tokio", "sqlx", "reqwest", "tantivy", "axum"] {
        assert!(
            !manifest.contains(&format!("{forbidden} =")),
            "found forbidden runtime dependency token: {forbidden}"
        );
        assert!(
            !prelude.contains(forbidden),
            "found forbidden runtime token in source: {forbidden}"
        );
    }

    for forbidden in ["unwrap(", "expect(", "panic!("] {
        assert!(
            !prelude.contains(forbidden),
            "found forbidden failure path token: {forbidden}"
        );
    }
}
