use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::{
    ApprovalDecision, DomainEvent, DomainInput, EvidenceId, HarnessRunId, KernelState,
    ModelAgentProposalExecution, TaskId,
};
use maestria_ports::{ApprovalRecord, ApprovalRepository, EffectJournal, ModelAgentProposal};
use maestria_storage_sqlite::SqliteStore;

use super::super::protocol::{
    ClientResponse, ModelAgentHarnessOutcome, ModelAgentMemoryCandidateSummary,
    ModelAgentProposalPayload, ModelAgentProposalResponse, ModelAgentStatusResponse,
    ModelAgentValidationSummary,
};
use super::super::server::ApiContext;

pub(super) async fn propose(
    context: &ApiContext,
    payload: ModelAgentProposalPayload,
) -> Result<ClientResponse> {
    let task_validation = payload.task_validation;
    let memory_candidate = payload.memory_candidate;
    let proposal = build_proposal(payload);
    let state = crate::instance_setup::load_kernel_state(&context.layout)
        .with_context(|| "load kernel state for proposal validation")?;
    validate_proposal_against_state(&proposal, &state)?;
    let Some(runtime) = context.runtime.clone() else {
        return Err(anyhow!(
            "model-agent proposal requires the canonical runtime command path"
        ));
    };
    let result = runtime
        .submit(DomainInput::ModelAgentProposalRequested(
            maestria_domain::ModelAgentProposalRequest {
                run_id: proposal.run_id,
                task_id: proposal.task_id,
                query: proposal.query,
                limit: proposal.limit,
                evidence_ids: proposal.evidence_ids.clone(),
                capability: proposal.capability,
                command: proposal.command,
                working_directory: proposal.working_directory.display().to_string(),
                timeout_secs: proposal.timeout.as_secs(),
                expected_generation: maestria_domain::IndexGenerationId::new(
                    proposal.expected_generation,
                ),
                task_validation,
                memory_candidate,
                execution: ModelAgentProposalExecution::Fresh,
                correlation_id: maestria_domain::CorrelationId::new(0),
            },
        ))
        .await
        .map_err(|error| anyhow!("model-agent proposal was not accepted: {error}"))?;

    Ok(ClientResponse::ModelAgentProposal(
        ModelAgentProposalResponse {
            run_id: proposal.run_id.value(),
            correlation_id: result.correlation_id,
            status: "accepted".to_string(),
            approval_id: None,
            trace_id: None,
            index_generation: current_generation(&state),
            evidence_count: proposal.evidence_ids.len(),
            harness: None,
            validation: None,
            memory_candidate: None,
            warnings: vec![format!(
                "runtime accepted proposal correlation {} with deferred query, validation, \
                 memory, and harness outcomes; use model_agent_status",
                result.correlation_id
            )],
        },
    ))
}

pub(super) async fn resolve(
    context: &ApiContext,
    run_id: u64,
    approval_id: u64,
    approved: bool,
) -> Result<ClientResponse> {
    let store = SqliteStore::open_read_only(&context.layout.database_path)?;
    let record = store
        .find_by_id(maestria_domain::ApprovalId::new(approval_id))?
        .ok_or_else(|| anyhow!("model-agent approval {approval_id} does not exist"))?;
    let identity = pending_proposal_identity(&record)
        .map_err(|error| anyhow!("approval {approval_id}: {error}"))?
        .ok_or_else(|| anyhow!("approval {approval_id} is not a model-agent proposal"))?;
    let pending_run_id = identity.run_id;
    let correlation_id = identity.correlation_id;
    if pending_run_id != run_id {
        return Err(anyhow!(
            "approval {approval_id} belongs to model-agent run {pending_run_id}, not {run_id}"
        ));
    }
    let Some(runtime) = context.runtime.clone() else {
        return Err(anyhow!(
            "model-agent approval requires the canonical runtime command path"
        ));
    };
    // Model-agent approvals record the outcome without transitioning a task;
    // the task linkage is audit metadata on the acknowledgement.
    let decision = ApprovalDecision::Acknowledge {
        approval_id: record.id,
        task_id: record.task_id,
        approved,
    };
    runtime
        .submit(DomainInput::ApprovalResolved(decision))
        .await
        .map_err(|error| anyhow!("model-agent approval was not accepted: {error}"))?;
    Ok(ClientResponse::ModelAgentStatus(ModelAgentStatusResponse {
        run_id,
        correlation_id: Some(correlation_id.value()),
        status: if approved {
            "approval_recorded"
        } else {
            "denial_recorded"
        }
        .to_string(),
        approval_id: Some(approval_id),
        journal_generation: None,
        trace_id: None,
        evidence_count: 0,
        harness: None,
        validation: None,
        memory_candidate: None,
        error: None,
    }))
}

/// Build the terminal status response for a completed proposal result.
fn terminal_response(
    run_id: u64,
    result: &maestria_domain::ModelAgentProposalResult,
) -> ModelAgentStatusResponse {
    match result {
        maestria_domain::ModelAgentProposalResult::Succeeded {
            search,
            harness,
            validation,
            memory_candidate,
            ..
        } => ModelAgentStatusResponse {
            run_id,
            correlation_id: Some(result.correlation_id().value()),
            status: "succeeded".to_string(),
            approval_id: None,
            journal_generation: None,
            trace_id: search.as_ref().map(|search| search.trace_id.value()),
            evidence_count: search.as_ref().map_or(0, |search| search.evidence_count),
            harness: harness.as_ref().map(|harness| ModelAgentHarnessOutcome {
                exit_code: harness.exit_code,
                stdout: harness.stdout.clone(),
                stderr: harness.stderr.clone(),
                duration_ms: harness.duration_ms,
            }),
            validation: validation
                .as_ref()
                .map(|validation| ModelAgentValidationSummary {
                    passed: validation.passed,
                    warnings: validation.warnings.clone(),
                }),
            memory_candidate: memory_candidate.as_ref().map(|memory| {
                ModelAgentMemoryCandidateSummary {
                    candidate_id: memory.candidate_id.value(),
                    confidence_milli: memory.confidence_milli,
                    decision: memory.decision.as_str().to_string(),
                }
            }),
            error: None,
        },
        maestria_domain::ModelAgentProposalResult::Failed { error, .. } => {
            ModelAgentStatusResponse {
                run_id,
                correlation_id: Some(result.correlation_id().value()),
                status: "failed".to_string(),
                approval_id: None,
                journal_generation: None,
                trace_id: None,
                evidence_count: 0,
                harness: None,
                validation: None,
                memory_candidate: None,
                error: Some(error.clone()),
            }
        }
    }
}

pub(super) fn status(layout: &InstanceLayout, run_id: u64) -> Result<ModelAgentStatusResponse> {
    let state = crate::instance_setup::load_kernel_state(layout)
        .map_err(|error| anyhow!("load model-agent terminal result state: {error:#}"))?;
    if let Some(result) = state
        .model_agent_results
        .get(&maestria_domain::HarnessRunId::new(run_id))
    {
        return Ok(terminal_response(run_id, result));
    }
    let store = SqliteStore::open_read_only(&layout.database_path)?;
    let mut approval_id = None;
    let mut correlation_id = None;
    let mut pending_journal_generation = None;
    for record in store.find_pending()? {
        let Some(identity) = pending_proposal_identity(&record)
            .map_err(|error| anyhow!("read approval {}: {error}", record.id))?
        else {
            continue;
        };
        if identity.run_id == run_id {
            approval_id = Some(record.id.value());
            correlation_id = Some(identity.correlation_id.value());
            pending_journal_generation = Some(identity.journal_generation.value());
            break;
        }
    }
    let journal = store.scan_in_flight()?;
    let entry = journal.iter().find(|entry| {
        entry.run_id.value() == run_id
            && pending_journal_generation.is_none_or(|generation| entry.generation == generation)
    });
    let status = if approval_id.is_some() {
        "pending_approval"
    } else if entry.is_some() {
        "running"
    } else {
        "submitted"
    };
    Ok(ModelAgentStatusResponse {
        run_id,
        correlation_id,
        status: status.to_string(),
        approval_id,
        journal_generation: pending_journal_generation
            .or_else(|| entry.map(|entry| entry.generation)),
        trace_id: None,
        evidence_count: 0,
        harness: None,
        validation: None,
        memory_candidate: None,
        error: None,
    })
}

fn current_generation(state: &KernelState) -> u64 {
    state
        .event_log
        .iter()
        .filter_map(|env| match &env.event {
            DomainEvent::IndexGenerationStarted { id, .. } => Some(id.value()),
            _ => None,
        })
        .fold(0, u64::max)
}

fn build_proposal(payload: ModelAgentProposalPayload) -> ModelAgentProposal {
    let run_id = HarnessRunId::new(payload.run_id);
    let task_id = payload.task_id.map(TaskId::new);
    let evidence_ids: Vec<EvidenceId> = payload
        .evidence_ids
        .iter()
        .map(|id| EvidenceId::new(*id))
        .collect();
    let working_directory = std::path::PathBuf::from(&payload.working_directory);
    let timeout = Duration::from_secs(payload.timeout_secs);

    ModelAgentProposal {
        run_id,
        task_id,
        query: payload.query,
        limit: payload.limit,
        capability: payload.capability,
        command: payload.command,
        working_directory,
        timeout,
        expected_generation: payload.expected_generation,
        evidence_ids,
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingProposalIdentity {
    run_id: u64,
    correlation_id: maestria_domain::CorrelationId,
    journal_generation: maestria_domain::JournalGeneration,
}

/// Decode the pending continuation identity of an approval record.
///
/// Returns `Ok(None)` for records without a pending model-agent continuation
/// and `Err` when a record claims one but its token is corrupt, so the status
/// surface reports corruption instead of silently skipping the record.
fn pending_proposal_identity(
    record: &ApprovalRecord,
) -> Result<Option<PendingProposalIdentity>, String> {
    let Some(proposal) = maestria_runtime::decode_pending_continuation(record)? else {
        return Ok(None);
    };
    let Some(journal_generation) = proposal.execution.journal_generation() else {
        return Ok(None);
    };
    Ok(Some(PendingProposalIdentity {
        run_id: proposal.run_id.value(),
        correlation_id: proposal.correlation_id,
        journal_generation,
    }))
}

fn validate_proposal_against_state(
    proposal: &ModelAgentProposal,
    state: &KernelState,
) -> Result<maestria_ports::GovernedAgentProposal> {
    let cur_gen = current_generation(state);
    let available_evidence: BTreeSet<EvidenceId> = state.evidences.keys().copied().collect();

    proposal
        .validate(cur_gen, &available_evidence)
        .map_err(anyhow::Error::new)
}
