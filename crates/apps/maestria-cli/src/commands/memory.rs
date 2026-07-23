use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::{DomainInput, MemoryCandidate, ProposeMemoryCandidateInput};
use maestria_governance::{
    AutonomyProfile, DefaultMemoryPromotionGate, MemoryPromotionDecision, MemoryPromotionGate,
    MemoryPromotionRequest,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::{sleep, timeout};

use crate::helpers;

pub fn run(instance_dir: PathBuf, limit: usize) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
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
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let state = load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before memory proposal",
    )
    .await?;

    // Pre-validate evidence existence.
    for &eid in &evidence_ids {
        let eid = maestria_domain::EvidenceId::new(eid);
        if !state.evidences.contains_key(&eid) {
            return Err(anyhow!("evidence {eid} not found"));
        }
    }
    let (runtime, input_tx, input_rx, shutdown_token) = timeout(Duration::from_secs(5), async {
        loop {
            match maestria_daemon::build_runtime(
                &layout,
                state.clone(),
                AutonomyProfile::TrustedWorkspace,
            ) {
                Ok(runtime) => break Ok(runtime),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => break Err(error).with_context(|| "build runtime"),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while building runtime"))??;

    let (claim_id, candidate_id) = timeout(Duration::from_secs(5), async {
        loop {
            match runtime.allocate_memory_proposal_ids() {
                Ok(ids) => break Ok(ids),
                Err(error) => {
                    let error = anyhow::Error::from(error);
                    if helpers::is_db_locked(&error) {
                        sleep(Duration::from_millis(25)).await;
                    } else {
                        break Err(error).with_context(|| "allocate memory proposal ids");
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while allocating memory proposal ids"))??;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
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
        input_tx
            .send(input)
            .await
            .map_err(|error| anyhow::anyhow!("failed to queue proposal input: {error}"))?;
        wait_for_candidate(&layout, candidate_id, Duration::from_secs(5)).await
    }
    .await;

    shutdown_token.cancel();
    let join_result = runtime_task.await;
    let state = result?;
    join_result.with_context(|| "runtime loop join failed")?;

    let candidate = state
        .memory_candidates
        .get(&candidate_id)
        .cloned()
        .ok_or_else(|| anyhow!("memory candidate {} was not persisted", candidate_id))?;

    println!(
        "proposed candidate={} claim={} confidence={}/1000 evidence={}",
        candidate.id,
        candidate.claim_id,
        candidate.confidence_milli,
        candidate.evidence_ids.len(),
    );

    Ok(())
}

pub async fn run_promote(
    instance_dir: PathBuf,
    candidate_id: u64,
    user_approved: bool,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;

    let state = load_kernel_state_with_retry(
        &layout,
        Duration::from_secs(2),
        "load kernel state before memory promotion",
    )
    .await?;

    let candidate_id = maestria_domain::MemoryCandidateId::new(candidate_id);
    let candidate = state
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

    let memory_id = next_memory_id(&state);

    let (runtime, input_tx, input_rx, shutdown_token) = timeout(Duration::from_secs(5), async {
        loop {
            match maestria_daemon::build_runtime(
                &layout,
                state.clone(),
                AutonomyProfile::TrustedWorkspace,
            ) {
                Ok(runtime) => break Ok(runtime),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => break Err(error).with_context(|| "build runtime"),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while building runtime"))??;
    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
        input_tx
            .send(DomainInput::PromoteMemory(
                maestria_domain::PromoteMemoryInput {
                    memory_id,
                    candidate_id,
                },
            ))
            .await
            .map_err(|error| anyhow!("failed to queue memory promotion input: {error}"))?;
        wait_for_memory(&layout, memory_id, Duration::from_secs(5)).await
    }
    .await;

    let join_result = runtime_task.await;
    result?;
    join_result.with_context(|| "runtime loop join failed")?;

    println!("promoted candidate={candidate_id} memory={memory_id}");
    Ok(())
}

fn print_memory_candidate(candidate: &MemoryCandidate) {
    println!(
        "candidate={} claim={} confidence={} evidence={} ids={:?}",
        candidate.id,
        candidate.claim_id,
        candidate.confidence_milli,
        candidate.evidence_ids.len(),
        candidate.evidence_ids
    );
}
fn next_memory_id(state: &maestria_domain::KernelState) -> maestria_domain::MemoryId {
    state
        .memories
        .iter()
        .next_back()
        .map_or(maestria_domain::MemoryId::new(1), |(id, _)| {
            maestria_domain::MemoryId::new(id.value() + 1)
        })
}

async fn wait_for_memory(
    layout: &InstanceLayout,
    memory_id: maestria_domain::MemoryId,
    timeout_budget: Duration,
) -> Result<maestria_domain::KernelState> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout) {
                Ok(state) if state.memories.contains_key(&memory_id) => return Ok(state),
                Ok(_) => sleep(Duration::from_millis(25)).await,
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out waiting for promoted memory {memory_id}"))?
}

async fn wait_for_candidate(
    layout: &InstanceLayout,
    candidate_id: maestria_domain::MemoryCandidateId,
    timeout_budget: Duration,
) -> Result<maestria_domain::KernelState> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout) {
                Ok(state) if state.memory_candidates.contains_key(&candidate_id) => {
                    return Ok(state);
                }
                Ok(_) => sleep(Duration::from_millis(25)).await,
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out waiting for candidate {candidate_id}"))?
}

async fn load_kernel_state_with_retry(
    layout: &InstanceLayout,
    timeout_budget: Duration,
    context: &'static str,
) -> Result<maestria_domain::KernelState> {
    timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout).with_context(|| context) {
                Ok(state) => return Ok(state),
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("timed out while {context}"))?
}
