use maestria_domain::{
    ApprovalDecision, ApprovalId, DomainInput, KernelState, TaskId, TaskPriority, TaskStatus,
};
use maestria_ports::{ApprovalRepository, ApprovalStatus};
use std::collections::BTreeSet;

use maestria_storage_sqlite::SqliteStore;

use crate::reconcile_approval_repo;

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

#[test]
fn reconciliation_repairs_stale_repo_after_crash() {
    let store = SqliteStore::in_memory().expect("open in-memory store");
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    state.tasks.insert(task_id, make_task(1));
    let approval_id = ApprovalId::new(42);
    state
        .apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
            approval_id,
            task_id,
            approved: true,
        }))
        .expect("domain should accept approval input");
    let pending = store.find_pending().expect("find pending");
    assert!(pending.is_empty(), "repo empty before reconciliation");
    reconcile_approval_repo(&state, &store).expect("reconcile");
    let record = store.find_by_id(approval_id).expect("find").unwrap();
    assert_eq!(record.status, ApprovalStatus::Approved);
}

#[test]
fn reconciliation_handles_denied_approval() {
    let store = SqliteStore::in_memory().expect("open in-memory store");
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    let mut task = make_task(1);
    task.status = TaskStatus::Blocked;
    state.tasks.insert(task_id, task);
    let approval_id = ApprovalId::new(7);
    state
        .apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
            approval_id,
            task_id,
            approved: false,
        }))
        .expect("domain should accept denied approval");
    reconcile_approval_repo(&state, &store).expect("reconcile");
    let record = store.find_by_id(approval_id).expect("find").unwrap();
    assert_eq!(record.status, ApprovalStatus::Denied);
}

#[test]
fn reconciliation_idempotent_across_restarts() {
    let store = SqliteStore::in_memory().expect("open in-memory store");
    let mut state = KernelState::new();
    let task_id = TaskId::new(1);
    state.tasks.insert(task_id, make_task(1));
    let approval_id = ApprovalId::new(1);
    state
        .apply_input(DomainInput::ApprovalResolved(ApprovalDecision {
            approval_id,
            task_id,
            approved: true,
        }))
        .expect("first resolution");
    reconcile_approval_repo(&state, &store).expect("first reconcile");
    let record = store.find_by_id(approval_id).expect("find").unwrap();
    assert_eq!(record.status, ApprovalStatus::Approved);
    reconcile_approval_repo(&state, &store).expect("second reconcile");
    let record2 = store.find_by_id(approval_id).expect("find").unwrap();
    assert_eq!(record2.status, ApprovalStatus::Approved);
}
