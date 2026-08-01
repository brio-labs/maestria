use crate::EffectExecutionContext;
use crate::config::{Adapters, Governance};
use crate::effect_admission::EffectAdmission;
use crate::test_helpers;
use maestria_domain::{
    ApprovalId, DomainInput, HarnessRunId, KernelState, LogicalTick, MaestriaEffect,
    ModelAgentProposalExecution, ModelAgentProposalRequest, ScopeId, TaskId,
};
use maestria_governance::{ApprovalGate, ApprovalGateDecision, ApprovalRequest, PolicyDecision};
use maestria_ports::{
    ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus, EffectJournal,
    EffectJournalIntent, HarnessAdapter, InMemoryApprovalRepository, InMemoryEffectJournal,
    PortError,
};
use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{RwLock, mpsc};

#[derive(Debug)]
pub(super) struct ErrorApprovalRepository;

impl ApprovalRepository for ErrorApprovalRepository {
    fn save(&self, _record: &ApprovalRecord) -> Result<(), PortError> {
        Err(PortError::Internal {
            message: "approval lookup test failure".to_string(),
        })
    }

    fn find_pending(&self) -> Result<Vec<ApprovalRecord>, PortError> {
        Err(PortError::Internal {
            message: "approval lookup test failure".to_string(),
        })
    }

    fn find_all(&self) -> Result<Vec<ApprovalRecord>, PortError> {
        Err(PortError::Internal {
            message: "approval lookup test failure".to_string(),
        })
    }

    fn find_by_id(&self, _id: ApprovalId) -> Result<Option<ApprovalRecord>, PortError> {
        Err(PortError::Internal {
            message: "approval lookup test failure".to_string(),
        })
    }

    fn resolve(
        &self,
        _id: ApprovalId,
        _approved: bool,
    ) -> Result<Option<ApprovalRecord>, PortError> {
        Err(PortError::Internal {
            message: "approval lookup test failure".to_string(),
        })
    }

    fn find_by_task_id(&self, _task_id: TaskId) -> Result<Vec<ApprovalRecord>, PortError> {
        Err(PortError::Internal {
            message: "approval lookup test failure".to_string(),
        })
    }
}

#[derive(Debug)]
struct FixedApprovalGate {
    decision: PolicyDecision,
}

impl ApprovalGate for FixedApprovalGate {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalGateDecision {
        ApprovalGateDecision {
            decision: self.decision.clone(),
            risk: request.risk,
        }
    }
}

pub(super) fn proposal(execution: ModelAgentProposalExecution) -> ModelAgentProposalRequest {
    ModelAgentProposalRequest {
        run_id: HarnessRunId::new(41),
        task_id: Some(TaskId::new(7)),
        query: "test query".to_string(),
        limit: 3,
        evidence_ids: Vec::new(),
        capability: "shell".to_string(),
        command: "echo approved".to_string(),
        working_directory: "/workspace".to_string(),
        timeout_secs: 10,
        expected_generation: maestria_domain::IndexGenerationId::new(1),
        task_validation: false,
        memory_candidate: false,
        execution,
        correlation_id: maestria_domain::CorrelationId::new(99),
    }
}

#[derive(serde::Serialize)]
struct ApprovalContinuationFixture<'a> {
    proposal: &'a ModelAgentProposalRequest,
    journal_generation: maestria_domain::JournalGeneration,
    correlation_id: maestria_domain::CorrelationId,
}

pub(super) fn approval_record(
    request: &ModelAgentProposalRequest,
    status: ApprovalStatus,
) -> Result<ApprovalRecord, Box<dyn std::error::Error>> {
    let journal_generation = request
        .execution
        .journal_generation()
        .ok_or("approval continuation fixture requires a journal generation")?;
    let approval_id = request
        .execution
        .approval_id()
        .ok_or("approval continuation fixture requires an approval id")?;
    let continuation = ApprovalContinuationFixture {
        proposal: request,
        journal_generation,
        correlation_id: request.correlation_id,
    };

    Ok(ApprovalRecord {
        id: approval_id,
        task_id: request.task_id,
        effect_kind: "model_agent_harness".to_string(),
        risk_level: ApprovalRiskLevel::High,
        capability: format!(
            "model_agent_pending:{}",
            serde_json::to_string(&continuation)?
        ),
        scope_id: ScopeId::new(1),
        tick: LogicalTick::new(0),
        status,
    })
}

#[derive(Debug)]
struct RecordingHarness {
    calls: Arc<AtomicUsize>,
}

impl HarnessAdapter for RecordingHarness {
    fn capabilities(&self) -> Result<maestria_ports::HarnessCapabilities, PortError> {
        Ok(maestria_ports::HarnessCapabilities {
            command_classes: vec![maestria_ports::HarnessCommandClass::Shell],
            write_enabled: true,
            read_enabled: true,
            web_enabled: false,
        })
    }

    fn execute(
        &self,
        request: maestria_ports::HarnessRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<maestria_ports::HarnessOutcome, PortError>> + Send + '_>,
    > {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(maestria_ports::HarnessOutcome {
                run_id: request.run_id,
                command: request.command,
                exit_code: 0,
                stdout: Vec::new(),
                stderr: Vec::new(),
                duration: std::time::Duration::from_millis(1),
                artifacts_created: Vec::new(),
                diff_summary: None,
                validation_hints: Vec::new(),
            })
        })
    }
}

type TestContext = (
    EffectExecutionContext,
    Arc<InMemoryEffectJournal>,
    mpsc::Receiver<DomainInput>,
);
type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

pub(super) fn context_with(
    approval_repo: Arc<dyn ApprovalRepository + Send + Sync>,
    governance: Arc<Governance>,
    harness_calls: Arc<AtomicUsize>,
) -> TestContext {
    let journal = Arc::new(InMemoryEffectJournal::default());
    let harness: Arc<dyn HarnessAdapter + Send + Sync> = Arc::new(RecordingHarness {
        calls: harness_calls,
    });
    let adapters = Arc::new(Adapters {
        approval_repo,
        effect_journal: journal.clone(),
        harness,
        ..test_helpers::test_adapters()
    });
    let (input_tx, input_rx) = mpsc::channel(16);
    let context = EffectExecutionContext::test_default(
        adapters,
        governance,
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );
    (context, journal, input_rx)
}

pub(super) fn default_context(
    approval_repo: Arc<dyn ApprovalRepository + Send + Sync>,
    harness_calls: Arc<AtomicUsize>,
) -> TestContext {
    context_with(
        approval_repo,
        Arc::new(test_helpers::test_governance()),
        harness_calls,
    )
}

pub(super) fn fixed_governance(decision: PolicyDecision) -> Arc<Governance> {
    let base = test_helpers::test_governance();
    Arc::new(Governance {
        classifier: base.classifier,
        approval_gate: Arc::new(FixedApprovalGate { decision }),
        validation_gate: base.validation_gate,
        memory_promotion_gate: base.memory_promotion_gate,
    })
}

pub(super) fn admit(context: &EffectExecutionContext, effect: &MaestriaEffect) -> EffectAdmission {
    context.admit_effect(effect)
}

pub(super) fn seed_intent(
    journal: &InMemoryEffectJournal,
    request: &ModelAgentProposalRequest,
    task_id: Option<TaskId>,
    capability: &str,
    command: &str,
    scope_id: ScopeId,
    generation: Option<u64>,
) -> Result<(), PortError> {
    journal.record_intent(EffectJournalIntent {
        run_id: request.run_id,
        task_id,
        capability: capability.to_string(),
        command: command.to_string(),
        scope_id,
        requested_generation: generation,
    })?;
    Ok(())
}

fn seed_canonical_fresh(
    context: &EffectExecutionContext,
    request: &ModelAgentProposalRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut canonical = request.clone();
    canonical.execution = ModelAgentProposalExecution::Fresh;
    let mut state = context
        .state
        .try_write()
        .map_err(|_| "canonical proposal state lock poisoned")?;
    state
        .model_agent_requests
        .insert(canonical.run_id, canonical);
    Ok(())
}

pub(super) fn seed_exact_approval(
    request: &ModelAgentProposalRequest,
    status: ApprovalStatus,
    calls: Arc<AtomicUsize>,
) -> TestResult<TestContext> {
    let repository = InMemoryApprovalRepository::new();
    repository.save(&approval_record(request, status)?)?;
    let (context, journal, receiver) = default_context(Arc::new(repository), calls);
    seed_canonical_fresh(&context, request)?;
    seed_intent(
        &journal,
        request,
        request.task_id,
        &request.capability,
        &request.command,
        ScopeId::new(1),
        request
            .execution
            .journal_generation()
            .map(|generation| generation.value()),
    )?;
    Ok((context, journal, receiver))
}

pub(super) fn assert_no_domain_input(receiver: &mut mpsc::Receiver<DomainInput>) {
    assert!(
        receiver.try_recv().is_err(),
        "rejection emitted a domain result"
    );
}

pub(super) fn recovery_context(
    request: &ModelAgentProposalRequest,
    calls: Arc<AtomicUsize>,
) -> TestResult<TestContext> {
    let (context, journal, receiver) =
        default_context(Arc::new(InMemoryApprovalRepository::new()), calls);
    seed_canonical_fresh(&context, request)?;
    let generation = request
        .execution
        .journal_generation()
        .ok_or("recovery fixture requires a journal generation")?;
    journal.record_intent(EffectJournalIntent {
        run_id: request.run_id,
        task_id: request.task_id,
        capability: request.capability.clone(),
        command: request.command.clone(),
        scope_id: ScopeId::new(1),
        requested_generation: Some(generation.value()),
    })?;
    journal.record_started(request.run_id, generation.value())?;
    journal.claim_feedback_with_outcome(
        request.run_id,
        generation.value(),
        maestria_ports::HarnessOutcome {
            run_id: request.run_id,
            command: request.command.clone(),
            exit_code: 0,
            stdout: b"recovered".to_vec(),
            stderr: Vec::new(),
            duration: std::time::Duration::from_millis(1),
            artifacts_created: Vec::new(),
            diff_summary: None,
            validation_hints: Vec::new(),
        },
    )?;
    Ok((context, journal, receiver))
}
