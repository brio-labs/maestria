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
        let task = req
            .task_id
            .map_or_else(|| "-".to_string(), |task_id| task_id.to_string());
        println!(
            "  ID: {}  Task: {}  Kind: {}  Risk: {:?}  Status: {:?}",
            req.id, task, req.effect_kind, req.risk_level, req.status
        );
    }
    println!();
    Ok(())
}

pub async fn run_resolve(instance_dir: PathBuf, id: u64, approved: bool) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let session = maestria_daemon::MutationSession::start(
        layout.clone(),
        maestria_governance::AutonomyProfile::TrustedWorkspace,
    )
    .await
    .context("start mutation session")?;

    let operation = async {
        let store = SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        let approval_id = maestria_domain::ApprovalId::new(id);
        let record = store
            .find_by_id(approval_id)
            .context("failed to query approval request")?
            .ok_or_else(|| anyhow::anyhow!("approval request {id} not found"))?;

        if record.status != maestria_ports::ApprovalStatus::Pending
            || session.state().resolved_approvals.contains(&approval_id)
        {
            anyhow::bail!(
                "approval request {id} is already resolved ({:?})",
                record.status
            );
        }

        let decision = match record.task_id {
            Some(task_id) => maestria_domain::ApprovalDecision::Resolve {
                approval_id,
                task_id,
                approved,
            },
            None => maestria_domain::ApprovalDecision::Acknowledge {
                approval_id,
                task_id: None,
                approved,
            },
        };

        session
            .submit(maestria_domain::DomainInput::ApprovalResolved(decision))
            .await?;

        Ok(record)
    }
    .await;

    let record = session.finish(operation).await?;
    let action = if approved { "Approved" } else { "Denied" };
    println!(
        "{action} approval request {id} for task {:?}.",
        record.task_id
    );
    Ok(())
}
