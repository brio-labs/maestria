use crate::test_support::*;
use maestria_domain::{
    ApprovalDecision, ApprovalId, ArtifactDetected, ArtifactId, DomainEvent, DomainInput,
    FetchWebRequest, FetchWebRequested, LogicalTick, ModelAgentProposalExecution,
    RegisterArtifactInput, ScopeId, content_hash,
};
use maestria_ports::{
    ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus, EventFilter, EventLog,
    InMemoryApprovalRepository, InMemoryEffectJournal, PortError,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct BlockingEventLog {
    inner: InMemoryEventLog,
    append_started: Arc<AtomicBool>,
    release_append: Arc<AtomicBool>,
}

impl EventLog for BlockingEventLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        if !self.append_started.swap(true, Ordering::SeqCst) {
            while !self.release_append.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
        }
        self.inner.append(event)
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        self.inner.scan(filter)
    }
}

#[derive(Clone)]
struct BlockingApprovalRepository {
    inner: InMemoryApprovalRepository,
    resolve_started: Arc<AtomicBool>,
    release_resolve: Arc<AtomicBool>,
}

impl ApprovalRepository for BlockingApprovalRepository {
    fn save(&self, record: &ApprovalRecord) -> Result<(), PortError> {
        self.inner.save(record)
    }

    fn find_pending(&self) -> Result<Vec<ApprovalRecord>, PortError> {
        self.inner.find_pending()
    }

    fn find_all(&self) -> Result<Vec<ApprovalRecord>, PortError> {
        self.inner.find_all()
    }

    fn find_by_id(&self, id: ApprovalId) -> Result<Option<ApprovalRecord>, PortError> {
        self.inner.find_by_id(id)
    }

    fn resolve(&self, id: ApprovalId, approved: bool) -> Result<Option<ApprovalRecord>, PortError> {
        self.resolve_started.store(true, Ordering::SeqCst);
        while !self.release_resolve.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        self.inner.resolve(id, approved)
    }

    fn find_by_task_id(
        &self,
        task_id: maestria_domain::TaskId,
    ) -> Result<Vec<ApprovalRecord>, PortError> {
        self.inner.find_by_task_id(task_id)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn approval_command_ack_waits_for_event_and_projection()
-> Result<(), Box<dyn std::error::Error>> {
    let approval_id = ApprovalId::new(77);
    let approval_inner = InMemoryApprovalRepository::new();
    approval_inner.save(&ApprovalRecord {
        id: approval_id,
        task_id: None,
        effect_kind: "shell".to_string(),
        risk_level: ApprovalRiskLevel::Low,
        capability: "shell".to_string(),
        scope_id: ScopeId::new(1),
        tick: LogicalTick::new(1),
        status: ApprovalStatus::Pending,
    })?;
    let resolve_started = Arc::new(AtomicBool::new(false));
    let release_resolve = Arc::new(AtomicBool::new(false));
    let approval_repo = Arc::new(BlockingApprovalRepository {
        inner: approval_inner,
        resolve_started: resolve_started.clone(),
        release_resolve: release_resolve.clone(),
    });
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        approval_repo: approval_repo.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            default_effect_timeout: Duration::from_secs(2),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));
    let submission = tokio::spawn(async move {
        handle
            .submit(DomainInput::ApprovalResolved(
                ApprovalDecision::Acknowledge {
                    approval_id,
                    task_id: None,
                    approved: true,
                },
            ))
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while !resolve_started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(!submission.is_finished());
    assert!(
        event_log
            .scan(EventFilter { artifact_id: None })?
            .iter()
            .any(|event| {
                matches!(
                    &event.event,
                    DomainEvent::ApprovalRecorded {
                        approval_id: id,
                        approved: true,
                        ..
                    } if *id == approval_id
                )
            })
    );
    let pending = approval_repo
        .find_by_id(approval_id)?
        .ok_or("approval disappeared before projection")?;
    assert_eq!(pending.status, ApprovalStatus::Pending);

    release_resolve.store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(Duration::from_secs(1), submission).await??;
    assert!(result.is_ok());
    let projected = approval_repo
        .find_by_id(approval_id)?
        .ok_or("approval disappeared after projection")?;
    assert_eq!(projected.status, ApprovalStatus::Approved);

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(1), run).await;
    Ok(())
}
#[tokio::test]
async fn parse_artifact_no_deadlock_at_max_concurrency_one()
-> Result<(), Box<dyn std::error::Error>> {
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            default_effect_timeout: Duration::from_secs(5),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        governance,
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    let source_bytes = b"fn main() {}".to_vec();
    let source_hash = content_hash(&source_bytes);
    let artifact_id = ArtifactId::new(1);

    // Send ArtifactDetected input — the domain loop produces a
    // ParseArtifact effect, whose handler enqueues ParserStarted and
    // then runs the persistence barrier. With max_concurrent_effects=1,
    // the barrier must not deadlock waiting for the PersistEvent.
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "deadlock-test".to_string(),
            source_path: "/repo/deadlock.rs".to_string(),
            source_bytes,
            content_hash: source_hash,
        }))
        .await?;

    // Wait for the ParserStarted event to be persisted (proves no deadlock).
    let barrier_passed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let mut events = Vec::new();
            if let Ok(scanned) = event_log.scan(EventFilter { artifact_id: None }) {
                events = scanned;
            }
            if events.iter().any(|e| {
                matches!(&e.event, DomainEvent::ParserStarted { artifact_id: id, .. } if *id == artifact_id)
            }) {
                break true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(1), run).await;

    let no_deadlock = matches!(barrier_passed, Ok(true));
    assert!(
        no_deadlock,
        "ParserStarted persistence barrier must not deadlock at max_concurrent_effects=1"
    );
    Ok(())
}
/// Ensure a full effect queue does not make the command loop hold the state
/// lock while an in-flight persistence effect needs to read that state.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn command_submission_progresses_while_persistence_reads_state()
-> Result<(), Box<dyn std::error::Error>> {
    let append_started = Arc::new(AtomicBool::new(false));
    let release_append = Arc::new(AtomicBool::new(false));
    let event_log = Arc::new(BlockingEventLog {
        inner: InMemoryEventLog::new(),
        append_started: append_started.clone(),
        release_append: release_append.clone(),
    });
    let adapters = Adapters {
        event_log,
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            input_buffer_size: 1,
            max_concurrent_effects: 1,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    let register = |artifact_id: u64| {
        DomainInput::RegisterArtifact(RegisterArtifactInput {
            artifact_id: ArtifactId::new(artifact_id),
            title: format!("artifact-{artifact_id}"),
            security: None,
        })
    };
    handle.submit(register(1)).await?;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !append_started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await?;

    handle.submit(register(2)).await?;
    let third = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.submit(register(3)).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let third_pending = !third.is_finished();

    release_append.store(true, Ordering::SeqCst);
    let third_result = tokio::time::timeout(Duration::from_secs(2), third).await??;
    third_result?;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(1), run).await;
    assert!(
        third_pending,
        "third command should wait for bounded effect capacity before release"
    );
    Ok(())
}

/// Verify that a failing effect _not_ handled inline (PersistEvent) goes
/// through the spawned executor path and that the runtime is cancelled when
/// the effect fails after all retries are exhausted. This exercises the
/// async supervisor boundary that previously silently discarded
/// EffectFailure values from spawned tasks.
#[tokio::test]
async fn spawned_effect_failure_propagates_to_supervisor_and_cancels_runtime()
-> Result<(), Box<dyn std::error::Error>> {
    // An unseeded InMemoryWebFetcher returns NotFound for any URL that
    // hasn't been seeded, so the FetchWeb effect (non-PersistEvent,
    // always spawned) will fail.
    let adapters = crate::test_helpers::test_adapters();
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            scope: Scope::new(vec![], vec![], vec![], vec![], true),
            default_effect_timeout: Duration::from_secs(1),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        governance,
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    // Trigger FetchWeb effect via domain input — the domain handler
    // produces a recognised FetchWeb effect, which the executor schedules
    // as a spawned task (not PersistEvent, so the spawned path).
    input_tx
        .send(DomainInput::FetchWebRequested(FetchWebRequested {
            request: FetchWebRequest {
                url: "https://example.com/missing".to_string(),
                max_bytes: 1024,
                max_requests: 1,
                max_latency_ms: 1000,
                allowed_domains: vec![],
                allowed_content_types: vec![],
            },
        }))
        .await?;

    // The runtime should shut down because the spawned FetchWeb effect
    // fails (no seeded URL) and now propagates the failure.
    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}

/// A pre-aborted JoinSet task must be supervised as a runtime failure rather
/// than disappearing during the executor's non-blocking reap.
#[tokio::test]
async fn pre_failed_spawned_task_cancels_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig::default(),
        KernelState::new(),
        crate::test_helpers::test_adapters(),
        crate::test_helpers::test_governance(),
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(
        runtime
            .test_with_pre_failed_effect_task()
            .run(input_rx, shutdown.clone()),
    );

    // Wake the executor after the injected task has been aborted. A normal
    // persistence effect keeps this test independent of network fixtures.
    input_tx
        .send(DomainInput::ClockTick(LogicalTick::new(1)))
        .await?;

    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}

// ── feedback capacity ─────────────────────────────────────────────────

#[test]
fn feedback_reports_capacity_without_waiting() -> Result<(), FeedbackError> {
    let config = RuntimeConfig {
        input_buffer_size: 1,
        ..RuntimeConfig::default()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        config,
        KernelState::new(),
        crate::test_helpers::test_adapters(),
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    handle.try_send_feedback(DomainInput::ClockTick(LogicalTick::new(1)))?;
    assert_eq!(
        handle.try_send_feedback(DomainInput::ClockTick(LogicalTick::new(2))),
        Err(FeedbackError::CapacityFull)
    );
    drop(input_rx);
    assert_eq!(
        handle.try_send_feedback(DomainInput::ClockTick(LogicalTick::new(3))),
        Err(FeedbackError::RuntimeShutdown)
    );
    Ok(())
}

#[tokio::test]
async fn approval_ack_includes_inline_continuation_admission_before_shutdown()
-> Result<(), Box<dyn std::error::Error>> {
    let approval_id = ApprovalId::new(91);
    let generation = 1;
    let proposal =
        super::admission_support::proposal(ModelAgentProposalExecution::ApprovalContinuation {
            approval_id,
            journal_generation: generation,
        });
    let approval_repo = InMemoryApprovalRepository::new();
    approval_repo.save(&super::admission_support::approval_record(
        &proposal,
        ApprovalStatus::Pending,
    )?)?;
    let journal = Arc::new(InMemoryEffectJournal::default());
    super::admission_support::seed_intent(
        &journal,
        &proposal,
        proposal.task_id,
        &proposal.capability,
        &proposal.command,
        ScopeId::new(1),
        Some(generation),
    )?;
    let mut canonical = proposal.clone();
    canonical.execution = ModelAgentProposalExecution::Fresh;
    let mut state = KernelState::new();
    state
        .model_agent_requests
        .insert(canonical.run_id, canonical);
    let adapters = Adapters {
        approval_repo: Arc::new(approval_repo),
        effect_journal: journal.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig::default(),
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let shutdown = CancellationToken::new();
    let run_shutdown = shutdown.clone();
    let run = tokio::spawn(runtime.with_graceful_shutdown().run(input_rx, run_shutdown));

    let application = handle
        .submit(DomainInput::ApprovalResolved(
            ApprovalDecision::Acknowledge {
                approval_id,
                task_id: proposal.task_id,
                approved: true,
            },
        ))
        .await?;
    assert_eq!(application.effects_admitted, 3);
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), run).await??;

    assert!(application.events.iter().any(|event| {
        matches!(
            &event.event,
            maestria_domain::DomainEvent::ModelAgentProposalCompleted { result }
                if result.run_id == proposal.run_id
        )
    }));
    Ok(())
}

#[tokio::test]
async fn approval_ack_propagates_inline_continuation_admission_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let approval_id = ApprovalId::new(92);
    let proposal =
        super::admission_support::proposal(ModelAgentProposalExecution::ApprovalContinuation {
            approval_id,
            journal_generation: 1,
        });
    let approval_repo = InMemoryApprovalRepository::new();
    approval_repo.save(&super::admission_support::approval_record(
        &proposal,
        ApprovalStatus::Pending,
    )?)?;
    let mut canonical = proposal.clone();
    canonical.execution = ModelAgentProposalExecution::Fresh;
    let mut state = KernelState::new();
    state
        .model_agent_requests
        .insert(canonical.run_id, canonical);
    let adapters = Adapters {
        approval_repo: Arc::new(approval_repo),
        effect_journal: Arc::new(InMemoryEffectJournal::default()),
        ..crate::test_helpers::test_adapters()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig::default(),
        state,
        adapters,
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let shutdown = CancellationToken::new();
    let run_shutdown = shutdown.clone();
    let run = tokio::spawn(runtime.with_graceful_shutdown().run(input_rx, run_shutdown));

    let result = handle
        .submit(DomainInput::ApprovalResolved(
            ApprovalDecision::Acknowledge {
                approval_id,
                task_id: proposal.task_id,
                approved: true,
            },
        ))
        .await;
    match result {
        Err(crate::RuntimeSubmissionError::EffectPreparationRejected { reason, .. }) => {
            assert!(reason.contains("journal entry is missing"), "{reason}");
        }
        other => return Err(format!("unexpected approval result: {other:?}").into()),
    }
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), run).await??;
    Ok(())
}
