use anyhow::{Result, anyhow};

use maestria_domain::{KernelState, LogicalTick, ScopeId};
use maestria_ports::{
    ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus, IdAllocator,
};
use maestria_storage_sqlite::SqliteStore;

/// Reconcile the approval repository from replayed domain events.
///
/// After `load_kernel_state` replays the event log, this function scans for
/// `ApprovalRecorded` events and ensures the approval repository reflects the
/// resolved state. If a CLI-initiated resolution persisted the event but crashed
/// before updating the repo, this repair brings the repo back into consistency.
pub fn reconcile_approval_repo(state: &KernelState, store: &SqliteStore) -> Result<()> {
    use maestria_domain::DomainEvent;

    for envelope in &state.event_log {
        if let DomainEvent::ApprovalRecorded {
            approval_id,
            approved,
            ..
        } = &envelope.event
        {
            let existing = store
                .find_by_id(*approval_id)
                .map_err(|e| anyhow!("reconcile approval {approval_id}: {e}"))?;
            if existing.is_none() {
                anyhow::bail!(
                    "reconcile: approval record {approval_id} not found in repository; \
                     event log contains ApprovalRecorded but no matching durable request"
                );
            }
            if let Err(e) = store.resolve(*approval_id, *approved) {
                anyhow::bail!("reconcile: resolve approval {approval_id}: {e}");
            }
        }
    }
    Ok(())
}

/// Recreate missing approval requests for high-priority tasks that lost their
/// durable request due to a crash between TaskOpened persistence and
/// approval_repo.save(). Scans kernel state for Draft/Open tasks with high
/// priority, checks if any approval record exists for that task ID, and if
/// not, creates a new pending request. Tasks with an existing Denied record
/// are skipped (denial is terminal).
pub fn reconcile_pending_approvals(
    state: &KernelState,
    store: &SqliteStore,
    id_allocator: &dyn IdAllocator,
) -> Result<()> {
    use maestria_domain::TaskPriority;

    for task in state.tasks.values() {
        if task.priority != TaskPriority::High {
            continue;
        }
        if !matches!(
            task.status,
            maestria_domain::TaskStatus::Draft | maestria_domain::TaskStatus::Open
        ) {
            continue;
        }

        let existing = store
            .find_by_task_id(task.id)
            .map_err(|e| anyhow!("find approvals for task {}: {e}", task.id))?;
        if !existing.is_empty() {
            continue;
        }

        let approval_id = id_allocator
            .allocate_approval_id()
            .map_err(|e| anyhow!("allocate approval id for task {}: {e}", task.id))?;
        let record = ApprovalRecord {
            id: approval_id,
            task_id: task.id,
            effect_kind: "task_activation".to_string(),
            risk_level: ApprovalRiskLevel::Medium,
            capability: "task_activation".to_string(),
            scope_id: ScopeId::new(1),
            tick: LogicalTick::new(0),
            status: ApprovalStatus::Pending,
        };
        store
            .save(&record)
            .map_err(|e| anyhow!("save recreated approval for task {}: {e}", task.id))?;
    }
    Ok(())
}
