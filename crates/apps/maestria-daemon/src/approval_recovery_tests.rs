use maestria_domain::{
    ApprovalDecision, ApprovalId, DomainInput, KernelState, LogicalTick, ScopeId, TaskId,
    TaskPriority, TaskStatus,
};
use maestria_ports::{ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus};
use std::collections::BTreeSet;

use maestria_storage_sqlite::SqliteStore;

use super::{reconcile_approval_repo, reconcile_pending_approvals};

fn make_task(id: u64) -> maestria_domain::Task {
    maestria_domain::Task {
        id: TaskId::new(id),
        title: "test task".into(),
        status: TaskStatus::Open,
        priority: TaskPriority::High,
        validation_report_id: None,
        artifact_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
    }
}

fn pending_record(id: u64, task_id: u64) -> ApprovalRecord {
    ApprovalRecord {
        id: ApprovalId::new(id),
        task_id: Some(TaskId::new(task_id)),
        effect_kind: "task_activation".to_string(),
        risk_level: ApprovalRiskLevel::Medium,
        capability: "task_activation".to_string(),
        scope_id: ScopeId::new(1),
        tick: LogicalTick::new(1),
        status: ApprovalStatus::Pending,
    }
}

#[test]
fn reconciliation_repairs_stale_repo_after_crash() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.save(&pending_record(42, 1))?;

    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    state.tasks.insert(task_id, make_task(1));
    let approval_id = ApprovalId::new(42);
    state.apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
        approval_id,
        task_id: Some(task_id),
        approved: true,
        affects_task: true,
    }))?;

    let pending = store.find_pending()?;
    assert_eq!(pending.len(), 1, "repo still pending before reconciliation");
    reconcile_approval_repo(&state, &store)?;
    let record = store
        .find_by_id(approval_id)?
        .ok_or_else(|| std::io::Error::other("approval record missing"))?;
    assert_eq!(record.status, ApprovalStatus::Approved);
    Ok(())
}

#[test]
fn reconciliation_handles_denied_approval() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.save(&pending_record(7, 1))?;

    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    let mut task = make_task(1);
    task.status = TaskStatus::Blocked;
    state.tasks.insert(task_id, task);
    state.apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
        approval_id: ApprovalId::new(7),
        task_id: Some(task_id),
        approved: false,
        affects_task: true,
    }))?;

    reconcile_approval_repo(&state, &store)?;
    let record = store
        .find_by_id(ApprovalId::new(7))?
        .ok_or("record missing")?;
    assert_eq!(record.status, ApprovalStatus::Denied);
    Ok(())
}

#[test]
fn reconciliation_idempotent_across_restarts() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.save(&pending_record(1, 1))?;

    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    state.tasks.insert(task_id, make_task(1));
    state.apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
        approval_id: ApprovalId::new(1),
        task_id: Some(task_id),
        approved: true,
        affects_task: true,
    }))?;

    reconcile_approval_repo(&state, &store)?;
    let record = store
        .find_by_id(ApprovalId::new(1))?
        .ok_or("record missing")?;
    assert_eq!(record.status, ApprovalStatus::Approved);

    reconcile_approval_repo(&state, &store)?;
    let record2 = store
        .find_by_id(ApprovalId::new(1))?
        .ok_or("record missing")?;
    assert_eq!(record2.status, ApprovalStatus::Approved);
    Ok(())
}

#[test]
fn reconciliation_errors_on_missing_record() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;

    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    state.tasks.insert(task_id, make_task(1));
    state.apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
        approval_id: ApprovalId::new(99),
        task_id: Some(task_id),
        approved: true,
        affects_task: true,
    }))?;

    let result = reconcile_approval_repo(&state, &store);
    assert!(
        result.is_err(),
        "reconciliation must error on missing record"
    );
    if let Err(err) = result {
        let err_str = err.to_string();
        assert!(err_str.contains("not found"));
    }
    Ok(())
}

#[test]
fn model_agent_approval_does_not_mask_task_activation_recovery()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.save(&ApprovalRecord {
        id: ApprovalId::new(9),
        task_id: Some(TaskId::new(1)),
        effect_kind: "model_agent_harness".to_string(),
        risk_level: ApprovalRiskLevel::High,
        capability: "shell".to_string(),
        scope_id: ScopeId::new(1),
        tick: LogicalTick::new(1),
        status: ApprovalStatus::Pending,
    })?;
    let mut state = KernelState::new();
    state.tasks.insert(TaskId::new(1), make_task(1));
    reconcile_pending_approvals(&state, &store, &store)?;
    let records = store.find_by_task_id(TaskId::new(1))?;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.effect_kind == "task_activation")
            .count(),
        1
    );
    Ok(())
}
