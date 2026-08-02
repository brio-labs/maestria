use std::time::Duration;

use anyhow::{Result, anyhow};
use maestria_domain::{EvidenceId, HarnessRunId, TaskId};
use maestria_ports::{ApprovalRepository, ModelAgentProposal};
use maestria_storage_sqlite::SqliteStore;

use super::super::protocol::{ClientResponse, ModelAgentProposalPayload};
use super::super::server::ApiContext;
use super::proposal_service;

/// Parse a proposal payload, validate it against durable state, and delegate
/// runtime dispatch and response assembly to `proposal_service` (R20).
pub(super) async fn propose(
    context: &ApiContext,
    payload: ModelAgentProposalPayload,
) -> Result<ClientResponse> {
    let task_validation = payload.task_validation;
    let memory_candidate = payload.memory_candidate;
    let proposal = build_proposal(payload);
    proposal_service::submit_proposal(context, proposal, task_validation, memory_candidate).await
}

/// Load the approval record for an id and delegate correlation checks, effect
/// dispatch, and response assembly to `proposal_service` (R20).
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
    proposal_service::submit_approval(context, record, run_id, approved).await
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
