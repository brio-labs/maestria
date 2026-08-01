use maestria_domain::*;

fn proposal(run_id: u64) -> ModelAgentProposalRequest {
    ModelAgentProposalRequest {
        run_id: HarnessRunId::new(run_id),
        task_id: Some(TaskId::new(7)),
        query: "find the answer".to_string(),
        limit: 3,
        evidence_ids: vec![EvidenceId::new(4)],
        capability: "model-agent".to_string(),
        command: "echo answer".to_string(),
        working_directory: "/tmp".to_string(),
        timeout_secs: 30,
        expected_generation: IndexGenerationId::new(11),
        task_validation: false,
        memory_candidate: false,
        execution: ModelAgentProposalExecution::Fresh,
        correlation_id: CorrelationId::new(42),
    }
}

fn result(run_id: u64) -> ModelAgentProposalResult {
    ModelAgentProposalResult::Succeeded {
        run_id: HarnessRunId::new(run_id),
        correlation_id: CorrelationId::new(42),
        search: None,
        harness: None,
        validation: None,
        memory_candidate: None,
    }
}

#[test]
fn proposal_request_is_persisted_before_async_effect() -> Result<(), DomainError> {
    let request = proposal(1);
    let mut state = KernelState::new();
    let output = state.apply_input(DomainInput::ModelAgentProposalRequested(request.clone()))?;

    assert_eq!(
        state.model_agent_requests.get(&request.run_id),
        Some(&request)
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::ModelAgentProposalRequested { request: event_request },
            ..
        }] if event_request == &request
    ));
    assert!(matches!(
        output.effects.as_slice(),
        [
            MaestriaEffect::PersistEvent { .. },
            MaestriaEffect::QueryHarnessProposal(_)
        ]
    ));
    Ok(())
}

#[test]
fn proposal_run_id_cannot_be_reused_after_request_or_completion() -> Result<(), DomainError> {
    let request = proposal(2);
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ModelAgentProposalRequested(request.clone()))?;
    assert!(matches!(
        state.apply_input(DomainInput::ModelAgentProposalRequested(request.clone())),
        Err(DomainError::DuplicateModelAgentProposalRunId { run_id }) if run_id == request.run_id
    ));

    state.apply_input(DomainInput::ModelAgentProposalCompleted(result(2)))?;
    assert!(!state.model_agent_requests.contains_key(&request.run_id));
    assert!(state.model_agent_results.contains_key(&request.run_id));
    assert!(matches!(
        state.apply_input(DomainInput::ModelAgentProposalCompleted(result(2))),
        Err(DomainError::DuplicateModelAgentProposalRunId { .. })
    ));
    assert!(matches!(
        state.apply_input(DomainInput::ModelAgentProposalRequested(request)),
        Err(DomainError::DuplicateModelAgentProposalRunId { .. })
    ));
    Ok(())
}

#[test]
fn requested_requires_fresh_execution_and_resume_binds_to_canonical_request()
-> Result<(), DomainError> {
    let mut state = KernelState::new();
    let request = proposal(4);
    let mut non_fresh = request.clone();
    non_fresh.execution = ModelAgentProposalExecution::JournalRecovery {
        journal_generation: JournalGeneration::new(1),
    };
    assert!(matches!(
        state.apply_input(DomainInput::ModelAgentProposalRequested(non_fresh)),
        Err(DomainError::ModelAgentProposalRequestNotFresh { run_id })
            if run_id == request.run_id
    ));

    state.apply_input(DomainInput::ModelAgentProposalRequested(request.clone()))?;
    assert!(matches!(
        state.apply_input(DomainInput::ModelAgentProposalResumed(request.clone())),
        Err(DomainError::ModelAgentProposalNotResumable { run_id })
            if run_id == request.run_id
    ));
    let mut resumed = request.clone();
    resumed.execution = ModelAgentProposalExecution::JournalRecovery {
        journal_generation: JournalGeneration::new(3),
    };
    let output = state.apply_input(DomainInput::ModelAgentProposalResumed(resumed))?;
    assert!(matches!(
        output.effects.as_slice(),
        [MaestriaEffect::QueryHarnessProposal(request)]
            if request.proposal.execution
                == ModelAgentProposalExecution::JournalRecovery {
                    journal_generation: JournalGeneration::new(3)
                }
    ));

    let mut mismatched = request;
    mismatched.command = "echo tampered".to_string();
    mismatched.execution = ModelAgentProposalExecution::JournalRecovery {
        journal_generation: JournalGeneration::new(3),
    };
    assert!(matches!(
        state.apply_input(DomainInput::ModelAgentProposalResumed(mismatched)),
        Err(DomainError::ModelAgentProposalResumeMismatch { run_id })
            if run_id == HarnessRunId::new(4)
    ));
    Ok(())
}

#[test]
fn proposal_replay_rejects_non_fresh_canonical_request() {
    let mut request = proposal(5);
    request.execution = ModelAgentProposalExecution::JournalRecovery {
        journal_generation: JournalGeneration::new(1),
    };
    let mut state = KernelState::new();
    let result = state.apply_event(DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ModelAgentProposalRequested {
            request: request.clone(),
        },
    });
    assert!(matches!(
        result,
        Err(DomainError::ModelAgentProposalRequestNotFresh { run_id })
            if run_id == request.run_id
    ));
}

#[test]
fn proposal_request_and_completion_replay_reconstruct_lifecycle() -> Result<(), DomainError> {
    let request = proposal(3);
    let mut source = KernelState::new();
    source.apply_input(DomainInput::ModelAgentProposalRequested(request.clone()))?;
    source.apply_input(DomainInput::ModelAgentProposalCompleted(result(3)))?;

    let mut replayed = KernelState::new();
    for envelope in source.event_log.clone() {
        replayed.apply_event(envelope)?;
    }
    assert!(replayed.model_agent_requests.is_empty());
    assert_eq!(
        replayed.model_agent_results.get(&request.run_id),
        Some(&result(3))
    );

    let mut outstanding = KernelState::new();
    outstanding.apply_event(source.event_log[0].clone())?;
    assert_eq!(
        outstanding.model_agent_requests.get(&request.run_id),
        Some(&request)
    );
    Ok(())
}

#[test]
fn proposal_execution_variants_keep_recovery_coordinates_typed() {
    let executions = [
        ModelAgentProposalExecution::Fresh,
        ModelAgentProposalExecution::JournalRecovery {
            journal_generation: JournalGeneration::new(11),
        },
        ModelAgentProposalExecution::ApprovalContinuation {
            approval_id: ApprovalId::new(2),
            journal_generation: JournalGeneration::new(12),
        },
    ];

    for execution in executions {
        match execution {
            ModelAgentProposalExecution::Fresh => {}
            ModelAgentProposalExecution::JournalRecovery { journal_generation } => {
                assert_eq!(journal_generation, JournalGeneration::new(11));
            }
            ModelAgentProposalExecution::ApprovalContinuation {
                approval_id,
                journal_generation,
            } => {
                assert_eq!(approval_id, ApprovalId::new(2));
                assert_eq!(journal_generation, JournalGeneration::new(12));
            }
        }
    }
}

#[test]
fn proposal_execution_variants_serde_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let executions = [
        ModelAgentProposalExecution::Fresh,
        ModelAgentProposalExecution::JournalRecovery {
            journal_generation: JournalGeneration::new(11),
        },
        ModelAgentProposalExecution::ApprovalContinuation {
            approval_id: ApprovalId::new(2),
            journal_generation: JournalGeneration::new(12),
        },
    ];

    for (run_id, execution) in executions.into_iter().enumerate() {
        let mut request = proposal(run_id as u64 + 4);
        request.execution = execution;
        let encoded = serde_json::to_string(&request)?;
        let decoded: ModelAgentProposalRequest = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, request);
    }
    Ok(())
}
