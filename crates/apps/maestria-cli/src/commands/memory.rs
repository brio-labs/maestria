use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::{DomainInput, MemoryCandidate, ProposeMemoryCandidateInput};
use maestria_governance::{
    AutonomyProfile, DefaultMemoryPromotionGate, MemoryPromotionDecision, MemoryPromotionGate,
    MemoryPromotionRequest,
};
use std::path::PathBuf;
use std::time::Duration;

use crate::helpers;

pub fn run(instance_dir: PathBuf, limit: usize) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let state = maestria_daemon::load_kernel_state(&layout).with_context(|| "load kernel state")?;

    if state.memory_candidates.is_empty() {
        println!("no memory candidates");
        return Ok(());
    }

    for candidate in state.memory_candidates.values().take(limit) {
        print_memory_candidate(candidate);
    }

    Ok(())
}

/// Propose a memory candidate under the instance mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A proposal already accepted by the
/// runtime may still reach durable state; inspect durable state before
/// retrying an interrupted command.
pub async fn run_propose(
    instance_dir: PathBuf,
    text: String,
    evidence_ids: Vec<u64>,
    confidence_milli: u16,
) -> Result<()> {
    if text.trim().is_empty() {
        return Err(anyhow!("claim text must not be empty"));
    }
    if evidence_ids.is_empty() {
        return Err(anyhow!("at least one evidence id is required"));
    }

    let layout = helpers::ensure_instance(instance_dir)?;
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await
            .context("start mutation session")?;

    let operation = async {
        let state = session.state();
        for &eid in &evidence_ids {
            let eid = maestria_domain::EvidenceId::new(eid);
            if !state.evidences.contains_key(&eid) {
                anyhow::bail!("evidence {eid} not found");
            }
        }

        let (claim_id, candidate_id) = session.allocate_memory_proposal_ids()?;
        let input = DomainInput::ProposeMemoryCandidate(ProposeMemoryCandidateInput {
            claim_id,
            candidate_id,
            text: text.clone(),
            evidence_ids: evidence_ids
                .iter()
                .map(|&id| maestria_domain::EvidenceId::new(id))
                .collect(),
            confidence_milli,
            security: None,
        });
        session.submit(input).await?;
        Ok(candidate_id)
    }
    .await;

    let candidate_id = session.finish(operation).await?;
    let state = wait_for_candidate(&layout, candidate_id, Duration::from_secs(5)).await?;
    let candidate = state
        .memory_candidates
        .get(&candidate_id)
        .cloned()
        .ok_or_else(|| anyhow!("memory candidate {} was not persisted", candidate_id))?;

    println!(
        "proposed candidate={} claim={} confidence={}/1000 evidence={}",
        candidate.id(),
        candidate.claim_id(),
        candidate.confidence_milli(),
        candidate.evidence_ids().len(),
    );

    Ok(())
}

/// Promote a memory candidate under the instance mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). A promotion already accepted by the
/// runtime may still reach durable state; inspect durable state before
/// retrying an interrupted command.
pub async fn run_promote(
    instance_dir: PathBuf,
    candidate_id: u64,
    user_approved: bool,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await
            .context("start mutation session")?;
    let candidate_id = maestria_domain::MemoryCandidateId::new(candidate_id);

    let operation = async {
        let candidate = session
            .state()
            .memory_candidates
            .get(&candidate_id)
            .cloned()
            .ok_or_else(|| anyhow!("memory candidate {candidate_id} not found"))?;

        let decision = DefaultMemoryPromotionGate.evaluate(&MemoryPromotionRequest {
            candidate,
            user_approved,
        });
        match decision {
            MemoryPromotionDecision::Promote => {}
            MemoryPromotionDecision::RequireEvidence { reason }
            | MemoryPromotionDecision::RequireReview { reason }
            | MemoryPromotionDecision::Deny { reason } => {
                anyhow::bail!("cannot promote memory candidate {candidate_id}: {reason}")
            }
        };

        let memory_id = session
            .state()
            .memories
            .iter()
            .next_back()
            .map_or(maestria_domain::MemoryId::new(1), |(id, _)| {
                maestria_domain::MemoryId::new(id.value() + 1)
            });

        session
            .submit(DomainInput::PromoteMemory(
                maestria_domain::PromoteMemoryInput {
                    memory_id,
                    candidate_id,
                },
            ))
            .await?;
        Ok(memory_id)
    }
    .await;

    let memory_id = session.finish(operation).await?;
    wait_for_memory(&layout, memory_id, Duration::from_secs(5)).await?;

    println!("promoted candidate={candidate_id} memory={memory_id}");
    Ok(())
}

fn print_memory_candidate(candidate: &MemoryCandidate) {
    println!(
        "candidate={} claim={} confidence={} evidence={} ids={:?}",
        candidate.id(),
        candidate.claim_id(),
        candidate.confidence_milli(),
        candidate.evidence_ids().len(),
        candidate.evidence_ids()
    );
}

async fn wait_for_memory(
    layout: &InstanceLayout,
    memory_id: maestria_domain::MemoryId,
    timeout_budget: Duration,
) -> Result<maestria_domain::KernelState> {
    helpers::wait_for_kernel_state(
        layout,
        timeout_budget,
        format!("waiting for promoted memory {memory_id}"),
        |state| state.memories.contains_key(&memory_id),
    )
    .await
}

async fn wait_for_candidate(
    layout: &InstanceLayout,
    candidate_id: maestria_domain::MemoryCandidateId,
    timeout_budget: Duration,
) -> Result<maestria_domain::KernelState> {
    helpers::wait_for_kernel_state(
        layout,
        timeout_budget,
        format!("waiting for candidate {candidate_id}"),
        |state| state.memory_candidates.contains_key(&candidate_id),
    )
    .await
}
