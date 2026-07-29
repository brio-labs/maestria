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
        expected_generation: 11,
        task_validation: false,
        memory_candidate: false,
        approval_id: Some(ApprovalId::new(2)),
        journal_generation: None,
        correlation_id: 42,
    }
}

fn result(run_id: u64) -> ModelAgentProposalResult {
    ModelAgentProposalResult {
        run_id: HarnessRunId::new(run_id),
        correlation_id: 42,
        status: ModelAgentTerminalStatus::Succeeded,
        search: None,
        harness: None,
        validation: None,
        memory_candidate: None,
        error: None,
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
