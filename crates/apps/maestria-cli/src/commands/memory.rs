use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use maestria_domain::MemoryCandidate;
use std::path::PathBuf;

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
