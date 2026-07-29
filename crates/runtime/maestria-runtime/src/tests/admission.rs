use super::admission_support::{
    ErrorApprovalRepository, admit, approval_record, assert_no_domain_input, context_with,
    default_context, fixed_governance, proposal, recovery_context, seed_exact_approval,
    seed_intent,
};
use crate::config::Adapters;
use crate::effect_admission::{EffectAdmission, RejectionCause, RejectionHandling};
use crate::effect_dispatch::EffectWork;
use crate::effect_result::EffectFailure;
use crate::test_helpers;
use maestria_domain::{
    ApprovalId, DomainEvent, DomainEventEnvelope, DomainInput, EventId, HarnessRunId, KernelState,
    LogicalTick, MaestriaEffect, ModelAgentProposalExecution, ModelAgentTerminalStatus,
    QueryHarnessProposalRequest, QueryHarnessRequest, RequestApprovalRequest, ScopeId,
    SequenceNumber, TaskId,
};
use maestria_governance::{AutonomyProfile, PolicyDecision};
use maestria_ports::{
    ApprovalRepository, ApprovalStatus, EffectJournal, EffectJournalIntent, EffectJournalStatus,
    InMemoryApprovalRepository, PortError,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn approval_repository_errors_are_rejected_typed_and_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(10),
        journal_generation: 2,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) =
        default_context(Arc::new(ErrorApprovalRepository), calls.clone());
    let effect =
        MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request });

    match admit(&context, &effect) {
        EffectAdmission::Rejected {
            cause: RejectionCause::ApprovalLookup(PortError::Internal { message }),
            handling: RejectionHandling::ObserveOnly,
            ..
        } => assert_eq!(message, "approval lookup test failure"),
        other => return Err(format!("unexpected admission: {other:?}").into()),
    }
    let result = context.execute_effect(effect, None).await;
    assert!(matches!(
        result,
        Err(EffectFailure::ApprovalLookup(PortError::Internal { message }))
            if message == "approval lookup test failure"
    ));
    assert!(journal.scan_in_flight()?.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_no_domain_input(&mut receiver);
    Ok(())
}

#[test]
fn fresh_execution_uses_generic_policy_without_authorization_coordinates()
-> Result<(), Box<dyn std::error::Error>> {
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, _) = context_with(
        Arc::new(InMemoryApprovalRepository::new()),
        fixed_governance(PolicyDecision::Allow),
        calls,
    );
    let request = proposal(ModelAgentProposalExecution::Fresh);
    match admit(
        &context,
        &MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest {
            proposal: request.clone(),
        }),
    ) {
        EffectAdmission::Execute { claim: None, .. } => {}
        other => return Err(format!("fresh proposal was not policy-admitted: {other:?}").into()),
    }
    assert!(journal.scan_in_flight()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn missing_and_malformed_stored_proposals_are_observe_only()
-> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        ("missing", None),
        ("malformed", Some("model_agent_pending:not-json")),
    ];
    for (label, malformed_capability) in cases {
        let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
            approval_id: ApprovalId::new(21),
            journal_generation: 2,
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let repository = InMemoryApprovalRepository::new();
        if let Some(capability) = malformed_capability {
            let mut record = approval_record(&request, ApprovalStatus::Approved)?;
            record.capability = capability.to_string();
            repository.save(&record)?;
        }
        let (context, journal, mut receiver) = default_context(Arc::new(repository), calls.clone());
        let effect =
            MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request });
        let result = context.execute_effect(effect, None).await;
        assert!(
            matches!(result, Err(EffectFailure::Denied(_))),
            "{label} proposal result: {result:?}"
        );
        assert!(
            journal.scan_in_flight()?.is_empty(),
            "{label} rejection mutated journal"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "{label} rejection invoked harness"
        );
        assert_no_domain_input(&mut receiver);
    }
    Ok(())
}

#[tokio::test]
async fn stored_identity_mismatch_is_observe_only_without_coordinates_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(22),
        journal_generation: 2,
    });
    let mut record = approval_record(&request, ApprovalStatus::Approved)?;
    record.scope_id = ScopeId::new(99);
    let repository = InMemoryApprovalRepository::new();
    repository.save(&record)?;
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) = default_context(Arc::new(repository), calls.clone());
    let result = context
        .execute_effect(
            MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request }),
            None,
        )
        .await;
    assert!(matches!(result, Err(EffectFailure::Denied(_))));
    assert!(journal.scan_in_flight()?.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_no_domain_input(&mut receiver);
    Ok(())
}

#[test]
fn approved_proposal_claim_requires_exact_journal_intent() -> Result<(), Box<dyn std::error::Error>>
{
    let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(23),
        journal_generation: 2,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, _) = seed_exact_approval(&request, ApprovalStatus::Approved, calls)?;
    match admit(
        &context,
        &MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest {
            proposal: request.clone(),
        }),
    ) {
        EffectAdmission::Execute {
            claim: Some(claim), ..
        } => {
            assert_eq!(claim.run_id, request.run_id);
            assert_eq!(claim.generation, 2);
        }
        other => return Err(format!("approved proposal lacked exact claim: {other:?}").into()),
    }
    let in_flight = journal.scan_in_flight()?;
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].status, EffectJournalStatus::Intent);
    Ok(())
}

#[test]
fn approved_proposals_reject_missing_mismatched_and_non_intent_journal_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(24),
        journal_generation: 2,
    });
    let metadata_cases = [
        (
            Some(TaskId::new(99)),
            "shell",
            "echo approved",
            ScopeId::new(1),
        ),
        (request.task_id, "browser", "echo approved", ScopeId::new(1)),
        (request.task_id, "shell", "echo altered", ScopeId::new(1)),
        (request.task_id, "shell", "echo approved", ScopeId::new(99)),
    ];
    for (task_id, capability, command, scope_id) in metadata_cases {
        let calls = Arc::new(AtomicUsize::new(0));
        let repository = InMemoryApprovalRepository::new();
        repository.save(&approval_record(&request, ApprovalStatus::Approved)?)?;
        let (context, journal, _) = default_context(Arc::new(repository), calls);
        seed_intent(
            &journal,
            &request,
            task_id,
            capability,
            command,
            scope_id,
            Some(2),
        )?;
        let admission = admit(
            &context,
            &MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest {
                proposal: request.clone(),
            }),
        );
        assert!(matches!(admission, EffectAdmission::Rejected { .. }));
    }

    for status in [
        EffectJournalStatus::Started,
        EffectJournalStatus::FeedbackAccepted,
        EffectJournalStatus::Completed,
        EffectJournalStatus::Failed,
        EffectJournalStatus::Paused,
        EffectJournalStatus::Superseded,
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let (context, journal, _) = seed_exact_approval(&request, ApprovalStatus::Approved, calls)?;
        match status {
            EffectJournalStatus::Started => journal.record_started(request.run_id, 2)?,
            EffectJournalStatus::FeedbackAccepted => journal.claim_feedback(request.run_id, 2)?,
            EffectJournalStatus::Completed
            | EffectJournalStatus::Failed
            | EffectJournalStatus::Paused => journal.record_terminal(request.run_id, 2, status)?,
            EffectJournalStatus::Superseded => {
                journal.record_intent(EffectJournalIntent {
                    run_id: request.run_id,
                    task_id: request.task_id,
                    capability: request.capability.clone(),
                    command: request.command.clone(),
                    scope_id: ScopeId::new(1),
                    requested_generation: Some(3),
                })?;
            }
            EffectJournalStatus::Intent => {
                return Err("Intent is the only admissible status".into());
            }
        }
        let admission = admit(
            &context,
            &MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest {
                proposal: request.clone(),
            }),
        );
        assert!(
            matches!(admission, EffectAdmission::Rejected { .. }),
            "status {status:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn approved_claim_happens_before_proposal_search_and_provider_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(25),
        journal_generation: 2,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) =
        seed_exact_approval(&request, ApprovalStatus::Approved, calls.clone())?;
    let result = context
        .execute_effect(
            MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request }),
            None,
        )
        .await;
    assert!(result.is_ok(), "unexpected result: {result:?}");
    let entries = journal.scan_in_flight()?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, EffectJournalStatus::Started);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        receiver.try_recv(),
        Ok(DomainInput::ModelAgentProposalCompleted(result))
            if result.status == ModelAgentTerminalStatus::Failed
    ));
    Ok(())
}

#[tokio::test]
async fn concurrent_approved_replays_allow_only_one_provider_call()
-> Result<(), Box<dyn std::error::Error>> {
    let mut request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(26),
        journal_generation: 2,
    });
    request.query.clear();
    request.working_directory.clear();
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, _journal, _receiver) =
        seed_exact_approval(&request, ApprovalStatus::Approved, calls.clone())?;
    let effect =
        MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request });
    let (first, second) = tokio::join!(
        context.clone().execute_effect(effect.clone(), None),
        context.execute_effect(effect, None),
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "approved replay crossed provider boundary twice"
    );
    assert_eq!(first.is_ok() as usize + second.is_ok() as usize, 1);
    Ok(())
}

#[tokio::test]
async fn exact_denied_stored_proposal_terminalizes_decoded_proposal()
-> Result<(), Box<dyn std::error::Error>> {
    let request = proposal(ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: ApprovalId::new(27),
        journal_generation: 2,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) =
        seed_exact_approval(&request, ApprovalStatus::Denied, calls)?;
    let effect = MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest {
        proposal: request.clone(),
    });
    let result = context.clone().execute_effect(effect.clone(), None).await;
    assert!(matches!(result, Err(EffectFailure::Denied(_))));
    assert!(journal.scan_in_flight()?.is_empty());
    match receiver.try_recv() {
        Ok(DomainInput::ModelAgentProposalCompleted(result)) => {
            assert_eq!(result.run_id, request.run_id);
            assert_eq!(result.correlation_id, request.correlation_id);
            assert_eq!(result.status, ModelAgentTerminalStatus::Failed);
        }
        other => {
            return Err(
                format!("stored denial did not emit decoded terminal result: {other:?}").into(),
            );
        }
    }
    let prepared = context
        .prepare_effect_before_reply(effect)
        .await
        .map_err(|error| error.to_string())?;
    context
        .execute_prepared(prepared, None)
        .await
        .map_err(|error| error.to_string())?;
    assert!(matches!(
        receiver.try_recv(),
        Ok(DomainInput::ModelAgentProposalCompleted(result))
            if result.run_id == request.run_id
                && result.status == ModelAgentTerminalStatus::Failed
    ));
    Ok(())
}

#[tokio::test]
async fn fresh_policy_denial_and_legacy_harness_denial_keep_trusted_terminalization()
-> Result<(), Box<dyn std::error::Error>> {
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) = context_with(
        Arc::new(InMemoryApprovalRepository::new()),
        fixed_governance(PolicyDecision::Deny {
            reason: "forced denial".to_string(),
        }),
        calls.clone(),
    );
    let fresh_request = proposal(ModelAgentProposalExecution::Fresh);
    let fresh = MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest {
        proposal: fresh_request.clone(),
    });
    assert!(matches!(
        context.clone().execute_effect(fresh, None).await,
        Err(EffectFailure::Denied(_))
    ));
    assert!(journal.scan_in_flight()?.is_empty());
    let generation_probe = journal.record_intent(EffectJournalIntent {
        run_id: fresh_request.run_id,
        task_id: fresh_request.task_id,
        capability: fresh_request.capability.clone(),
        command: fresh_request.command.clone(),
        scope_id: ScopeId::new(1),
        requested_generation: None,
    })?;
    assert_eq!(
        generation_probe.generation, 1,
        "fresh denial consumed or superseded a hidden journal generation"
    );
    journal.record_terminal(
        generation_probe.run_id,
        generation_probe.generation,
        EffectJournalStatus::Failed,
    )?;
    assert!(
        matches!(receiver.try_recv(), Ok(DomainInput::ModelAgentProposalCompleted(result)) if result.status == ModelAgentTerminalStatus::Failed)
    );

    let legacy = MaestriaEffect::QueryHarness(QueryHarnessRequest {
        run_id: HarnessRunId::new(52),
        task_id: None,
        generation: None,
        capability: "shell".to_string(),
        scope_id: ScopeId::new(1),
        approval_id: None,
        command: "echo denied".to_string(),
    });
    assert!(matches!(
        context.execute_effect(legacy, None).await,
        Err(EffectFailure::Denied(_))
    ));
    assert!(journal.scan_in_flight()?.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn request_approval_executes_once_for_every_profile_and_forced_gate()
-> Result<(), Box<dyn std::error::Error>> {
    let profiles = [
        AutonomyProfile::ReadOnly,
        AutonomyProfile::Assisted,
        AutonomyProfile::ScopedAutonomy,
        AutonomyProfile::StrictResearch,
        AutonomyProfile::TrustedWorkspace,
    ];
    let decisions = [
        PolicyDecision::Allow,
        PolicyDecision::RequireApproval {
            reason: "forced pending".to_string(),
        },
        PolicyDecision::Deny {
            reason: "forced deny".to_string(),
        },
    ];
    for profile in profiles {
        for decision in decisions.clone() {
            let repository = InMemoryApprovalRepository::new();
            let (mut context, _, _) = context_with(
                Arc::new(repository.clone()),
                fixed_governance(decision),
                Arc::new(AtomicUsize::new(0)),
            );
            context.profile = profile;
            let result = context
                .execute_effect(
                    MaestriaEffect::RequestApproval(RequestApprovalRequest {
                        task_id: TaskId::new(22),
                    }),
                    None,
                )
                .await;
            assert!(
                result.is_ok(),
                "request approval failed for {profile:?}: {result:?}"
            );
            let pending = repository.find_pending()?;
            assert_eq!(
                pending.len(),
                1,
                "expected exactly one pending record for {profile:?}"
            );
            assert_eq!(pending[0].status, ApprovalStatus::Pending);
            assert_eq!(pending[0].task_id, Some(TaskId::new(22)));
        }
    }
    Ok(())
}

#[tokio::test]
async fn persist_event_failure_cancels_effect_and_runtime_executors()
-> Result<(), Box<dyn std::error::Error>> {
    let adapters = Adapters {
        event_log: Arc::new(super::FailingEventLog),
        ..test_helpers::test_adapters()
    };
    let config = crate::RuntimeConfig {
        max_retries: 0,
        ..crate::RuntimeConfig::default()
    };
    let (runtime, _input_rx) = crate::MaestriaRuntime::new(
        config,
        KernelState::new(),
        adapters,
        test_helpers::test_governance(),
    );
    let (effect_tx, effect_rx) = mpsc::channel(1);
    let effect_shutdown = CancellationToken::new();
    let runtime_shutdown = CancellationToken::new();
    let executor =
        runtime.spawn_effect_executor(effect_rx, effect_shutdown.clone(), runtime_shutdown.clone());
    effect_tx
        .send(vec![EffectWork::Pending(MaestriaEffect::PersistEvent {
            envelope: Box::new(DomainEventEnvelope {
                id: EventId::new(1),
                sequence: SequenceNumber::new(1),
                event: DomainEvent::TickObserved {
                    at: LogicalTick::new(1),
                },
            }),
        })])
        .await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime_shutdown.cancelled(),
    )
    .await
    .map_err(|_| "runtime shutdown was not cancelled after persistence failure")?;
    assert!(effect_shutdown.is_cancelled());
    executor.await?;
    Ok(())
}

#[tokio::test]
async fn journal_recovery_canonical_mismatch_is_rejected_without_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let mut request = proposal(ModelAgentProposalExecution::JournalRecovery {
        journal_generation: 1,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) = recovery_context(&request, calls.clone())?;
    request.command = "echo tampered".to_string();
    let result = context
        .clone()
        .execute_effect(
            MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request }),
            None,
        )
        .await;
    assert!(matches!(result, Err(EffectFailure::Denied(_))));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_no_domain_input(&mut receiver);
    let claims = context
        .journal_recovery_claims
        .lock()
        .map_err(|_| "claim lock poisoned")?;
    assert!(claims.is_empty());
    assert_eq!(journal.scan_in_flight()?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn journal_recovery_claim_is_shared_atomic_and_prunes_stale_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let request = proposal(ModelAgentProposalExecution::JournalRecovery {
        journal_generation: 2,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (context, journal, mut receiver) = recovery_context(&request, calls.clone())?;
    {
        let mut claims = context
            .journal_recovery_claims
            .lock()
            .map_err(|_| "claim lock poisoned")?;
        claims.insert((HarnessRunId::new(999), 77));
    }
    let first = context.clone();
    let second = context.clone();
    let effect =
        MaestriaEffect::QueryHarnessProposal(QueryHarnessProposalRequest { proposal: request });
    let (left, right) = tokio::join!(
        first.execute_effect(effect.clone(), None),
        second.execute_effect(effect, None)
    );
    assert_eq!(
        usize::from(left.is_ok()) + usize::from(right.is_ok()),
        1,
        "exactly one recovery execution may claim the journal entry"
    );
    assert_eq!(
        usize::from(matches!(left, Err(EffectFailure::Denied(_))))
            + usize::from(matches!(right, Err(EffectFailure::Denied(_)))),
        1,
        "the duplicate must be denied before dispatch"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        receiver.recv().await,
        Some(DomainInput::HarnessRunCompleted(_))
    ));
    assert!(matches!(
        receiver.recv().await,
        Some(DomainInput::ModelAgentProposalCompleted(_))
    ));
    assert_no_domain_input(&mut receiver);
    let claims = context
        .journal_recovery_claims
        .lock()
        .map_err(|_| "claim lock poisoned")?;
    assert_eq!(claims.len(), 1);
    assert!(claims.contains(&(HarnessRunId::new(41), 2)));
    assert_eq!(journal.scan_in_flight()?.len(), 1);
    Ok(())
}
