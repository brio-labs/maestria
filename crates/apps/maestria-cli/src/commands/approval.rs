use anyhow::{Context, Result};
use std::path::PathBuf;

use maestria_ports::ApprovalRepository;
use maestria_storage_sqlite::SqliteStore;

use crate::helpers;

pub fn run_list(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;

    let pending = store
        .find_pending()
        .context("failed to query pending approval requests")?;

    if pending.is_empty() {
        println!("No pending approval requests.");
        return Ok(());
    }

    println!("Pending approval requests:\n");
    for req in &pending {
        println!(
            "  ID: {}  Task: {}  Kind: {}  Risk: {:?}  Status: {:?}",
            req.id, req.task_id, req.effect_kind, req.risk_level, req.status
        );
    }
    println!();
    Ok(())
}

pub async fn run_resolve(instance_dir: PathBuf, id: u64, approved: bool) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;

    // First validate the request exists and is pending
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let record = store
        .find_by_id(maestria_domain::ApprovalId::new(id))
        .context("failed to query approval request")?
        .ok_or_else(|| anyhow::anyhow!("approval request {id} not found"))?;

    if record.status != maestria_ports::ApprovalStatus::Pending {
        anyhow::bail!(
            "approval request {id} is already resolved ({:?})",
            record.status
        );
    }

    let lock = maestria_daemon::acquire_instance_write_lock(&layout)
        .await
        .context("acquire instance write lock")?;

    let state = maestria_daemon::load_kernel_state(&layout)
        .context("load kernel state")?;
    let profile = maestria_governance::AutonomyProfile::TrustedWorkspace;
    let (runtime, input_tx, mut input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, state, profile)
            .context("build runtime")?;

    let approval_id = maestria_domain::ApprovalId::new(id);
    let decision = maestria_domain::ApprovalDecision {
        approval_id,
        task_id: record.task_id,
        approved,
    };

    input_tx
        .send(maestria_domain::DomainInput::ApprovalResolved(decision))
        .await
        .context("send approval decision to runtime")?;

    // Wait a short time for the runtime to process the input
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if tokio::time::Instant::now() > deadline {
            break;
        }
        match tokio::time::timeout(
            std::time::Duration::from_millis(500),
            input_rx.recv(),
        )
        .await
        {
            Ok(Some(_)) | Ok(None) => break,
            Err(_) => continue,
        }
    }
    shutdown_token.cancel();
    drop(runtime);
    drop(lock);

    // Update the repository status
    let resolved = store
        .resolve(approval_id, approved)
        .context("failed to update approval request status")?;
    if resolved.is_none() {
        anyhow::bail!("approval request {id} was already resolved concurrently");
    }

    let action = if approved { "Approved" } else { "Denied" };
    println!("{action} approval request {id} for task {}.", record.task_id);
    Ok(())
}
