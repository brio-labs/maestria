use crate::config::EffectExecutionContext;
use crate::effect_result::EffectFailure;
use maestria_domain::{
    ApprovalId, LogicalTick, MaestriaEffect, ModelAgentProposalExecution,
    ModelAgentProposalRequest, QueryHarnessProposalRequest, QueryHarnessRequest,
};
use maestria_governance::{RiskClass, ScopeGuard};
use maestria_ports::{
    ApprovalRecord, ApprovalRiskLevel, ApprovalStatus, EffectJournalEntry, EffectJournalIntent,
    EffectJournalStatus,
};
use serde::{Deserialize, Serialize};

/// Continuation token payload stored on an approval record's capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingHarnessContinuation {
    proposal: ModelAgentProposalRequest,
    journal_generation: u64,
    correlation_id: u64,
}

fn pending_capability(token: &PendingHarnessContinuation) -> Result<String, EffectFailure> {
    serde_json::to_string(token)
        .map(|json| format!("model_agent_pending:{json}"))
        .map_err(|error| EffectFailure::Failed(format!("encode pending proposal: {error}")))
}

/// Decode a stored approval continuation token.
///
/// Returns `Ok(None)` when the record does not carry a pending model-agent
/// continuation (absence is a valid outcome). Returns `Err` when the record
/// claims to carry one but the token is corrupt, so callers can preserve the
/// failure instead of silently treating corrupted data as absence.
pub fn decode_pending_continuation(
    record: &ApprovalRecord,
) -> Result<Option<ModelAgentProposalRequest>, String> {
    let Some(json) = record.capability.strip_prefix("model_agent_pending:") else {
        return Ok(None);
    };
    let token: PendingHarnessContinuation = serde_json::from_str(json).map_err(|error| {
        format!(
            "decode pending model-agent continuation for approval {}: {error}",
            record.id
        )
    })?;
    let ModelAgentProposalExecution::ApprovalContinuation {
        approval_id: _,
        journal_generation,
    } = token.proposal.execution
    else {
        return Ok(None);
    };
    if token.journal_generation != journal_generation
        || token.correlation_id != token.proposal.correlation_id
    {
        return Ok(None);
    }
    Ok(Some(token.proposal))
}

/// Persist the pending state of a fresh harness proposal: journal the intent,
/// allocate an approval id, encode the continuation token, and store the
/// approval record awaiting external resolution.
pub(super) async fn persist_pending_harness(
    context: &EffectExecutionContext,
    request: &QueryHarnessProposalRequest,
) -> Result<(), EffectFailure> {
    let proposal = &request.proposal;
    if !matches!(&proposal.execution, ModelAgentProposalExecution::Fresh) {
        return Err(EffectFailure::Failed(
            "only a fresh proposal can create an approval continuation".to_string(),
        ));
    }
    let entry = record_harness_journal(context, proposal)?;
    let approval_id = context
        .adapters
        .id_allocator
        .allocate_approval_id()
        .map_err(|error| EffectFailure::Failed(format!("allocate harness approval id: {error}")))?;
    let capability = build_approval_continuation(proposal, approval_id, entry.generation)?;
    persist_approval_record(context, request, approval_id, capability).await?;
    tracing::info!(approval_id = %approval_id, correlation_id = proposal.correlation_id, "harness proposal pending approval");
    Ok(())
}

/// Record the intent of a pending harness proposal in the effect journal.
fn record_harness_journal(
    context: &EffectExecutionContext,
    proposal: &ModelAgentProposalRequest,
) -> Result<EffectJournalEntry, EffectFailure> {
    context
        .adapters
        .effect_journal
        .record_intent(EffectJournalIntent {
            run_id: proposal.run_id,
            task_id: proposal.task_id,
            capability: proposal.capability.clone(),
            command: proposal.command.clone(),
            scope_id: context.scope_id,
            requested_generation: None,
        })
        .map_err(|error| EffectFailure::Failed(format!("record pending harness intent: {error}")))
}

/// Encode the approval continuation token for a fresh harness proposal.
fn build_approval_continuation(
    proposal: &ModelAgentProposalRequest,
    approval_id: ApprovalId,
    journal_generation: u64,
) -> Result<String, EffectFailure> {
    let mut continuation = proposal.clone();
    continuation.execution = ModelAgentProposalExecution::ApprovalContinuation {
        approval_id,
        journal_generation,
    };
    let token = PendingHarnessContinuation {
        proposal: continuation,
        journal_generation,
        correlation_id: proposal.correlation_id,
    };
    pending_capability(&token)
}

/// Persist the pending approval record for a harness proposal.
async fn persist_approval_record(
    context: &EffectExecutionContext,
    request: &QueryHarnessProposalRequest,
    approval_id: ApprovalId,
    capability: String,
) -> Result<(), EffectFailure> {
    let proposal = &request.proposal;
    let tick = {
        let state = context.state.read().await;
        state
            .event_log
            .last()
            .map_or(0, |event| event.sequence.value())
    };
    let scope_guard = ScopeGuard::new(context.scope.clone());
    let risk = context.governance.classifier.classify(
        &MaestriaEffect::QueryHarnessProposal(request.clone()),
        &scope_guard,
    );
    let record = ApprovalRecord {
        id: approval_id,
        task_id: proposal.task_id,
        effect_kind: "model_agent_harness".to_string(),
        risk_level: risk_class_to_approval_risk_level(risk),
        capability,
        scope_id: context.scope_id,
        tick: LogicalTick::new(tick),
        status: ApprovalStatus::Pending,
    };
    context
        .adapters
        .approval_repo
        .save(&record)
        .map_err(|error| EffectFailure::Failed(format!("persist harness approval: {error}")))
}

pub(super) fn record_denied_harness(
    context: &EffectExecutionContext,
    request: &QueryHarnessRequest,
) -> Result<(), EffectFailure> {
    let entry = context
        .adapters
        .effect_journal
        .record_intent(EffectJournalIntent {
            run_id: request.run_id,
            task_id: request.task_id,
            capability: request.capability.clone(),
            command: request.command.clone(),
            scope_id: request.scope_id,
            requested_generation: request.generation,
        })
        .map_err(|error| EffectFailure::Failed(format!("record denied harness intent: {error}")))?;
    context
        .adapters
        .effect_journal
        .record_started(request.run_id, entry.generation)
        .and_then(|_| {
            context.adapters.effect_journal.record_terminal(
                request.run_id,
                entry.generation,
                EffectJournalStatus::Failed,
            )
        })
        .map_err(|error| EffectFailure::Failed(format!("record denied harness terminal: {error}")))
}

pub(super) fn risk_class_to_approval_risk_level(risk: RiskClass) -> ApprovalRiskLevel {
    match risk {
        RiskClass::Low => ApprovalRiskLevel::Low,
        RiskClass::Medium => ApprovalRiskLevel::Medium,
        RiskClass::High => ApprovalRiskLevel::High,
        RiskClass::Critical => ApprovalRiskLevel::Critical,
    }
}
