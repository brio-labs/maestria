use anyhow::{Context, Result};
use std::path::PathBuf;

use maestria_domain::DomainEvent;
use maestria_ports::{ApprovalRepository, EventFilter, EventLog};
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
    drop(store);

    let lock = maestria_daemon::acquire_instance_write_lock(&layout)
        .await
        .context("acquire instance write lock")?;

    let state = maestria_daemon::load_kernel_state(&layout)
        .context("load kernel state")?;
    let profile = maestria_governance::AutonomyProfile::TrustedWorkspace;
    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, state, profile)
            .context("build runtime")?;

    let approval_id = maestria_domain::ApprovalId::new(id);
    let decision = maestria_domain::ApprovalDecision {
        approval_id,
        task_id: record.task_id,
        approved,
    };

    let runtime_handle = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));
    tokio::task::yield_now().await;

    input_tx
        .send(maestria_domain::DomainInput::ApprovalResolved(decision))
        .await
        .context("send approval decision to runtime")?;

    // Allow the runtime to process the effect before shutting down.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    shutdown_token.cancel();
    let _ = runtime_handle.await;
    drop(lock);

    // Verify the ApprovalRecorded event was persisted.
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let events = store
        .scan(EventFilter { artifact_id: None })
        .context("scan event log")?;
    let event_observed = events.iter().any(|e| {
        matches!(
            &e.event,
            DomainEvent::ApprovalRecorded {
                approval_id: id,
                ..
            } if *id == approval_id
        )
    });

    if !event_observed {
        anyhow::bail!(
            "approval decision was not recorded by the domain; \
             the request may be processed on next daemon start"
        );
    }

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
