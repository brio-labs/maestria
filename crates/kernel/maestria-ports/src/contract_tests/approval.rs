//! Shared [`ApprovalRepository`] contract (Rule 25: every concrete approval
//! repository executes the shared persistence and resolution suite).
//!
//! Each assertion family uses a disjoint id range (1–8, 10–12, 20–21) so the
//! suite can run against a single shared repository instance.

use super::*;
use maestria_domain::{ApprovalId, LogicalTick, ScopeId, TaskId};

pub fn pending_record(id: u64) -> ApprovalRecord {
    ApprovalRecord {
        id: ApprovalId::new(id),
        task_id: Some(TaskId::new(100 + id)),
        effect_kind: "task_activation".to_string(),
        risk_level: ApprovalRiskLevel::Medium,
        capability: String::new(),
        scope_id: ScopeId::new(0),
        tick: LogicalTick::new(id),
        status: ApprovalStatus::Pending,
    }
}

/// Save, lookup by id, and task-scoped lookup round trips.
pub fn assert_approval_repository_round_trip(
    repository: &dyn ApprovalRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    repository.save(&pending_record(1))?;
    repository.save(&pending_record(2))?;

    let found = repository.find_by_id(ApprovalId::new(1))?;
    assert_eq!(
        found.as_ref().map(|record| record.id),
        Some(ApprovalId::new(1))
    );
    assert_eq!(
        found.as_ref().map(|record| record.status),
        Some(ApprovalStatus::Pending)
    );
    assert!(
        repository.find_by_id(ApprovalId::new(99))?.is_none(),
        "missing approval must resolve to None"
    );

    let for_task = repository.find_by_task_id(TaskId::new(101))?;
    assert_eq!(for_task.len(), 1);
    assert_eq!(for_task[0].id, ApprovalId::new(1));
    Ok(())
}

/// Pending scans expose only unresolved records.
pub fn assert_approval_repository_pending(
    repository: &dyn ApprovalRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    repository.save(&pending_record(10))?;
    repository.save(&pending_record(11))?;
    repository.save(&pending_record(12))?;
    repository.resolve(ApprovalId::new(11), true)?;

    let pending = repository.find_pending()?;
    let ids: Vec<u64> = pending.iter().map(|record| record.id.value()).collect();
    assert!(
        ids.contains(&10) && ids.contains(&12),
        "unresolved approvals must stay pending"
    );
    assert!(
        !ids.contains(&11),
        "resolved approval must leave the pending scan"
    );
    Ok(())
}

/// Resolution transitions pending records to approved or denied, is
/// idempotent for already-resolved records, and never fabricates records
/// for unknown ids.
pub fn assert_approval_repository_resolution(
    repository: &dyn ApprovalRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    repository.save(&pending_record(20))?;
    repository.save(&pending_record(21))?;

    let approved = repository.resolve(ApprovalId::new(20), true)?;
    assert_eq!(
        approved.map(|record| record.status),
        Some(ApprovalStatus::Approved)
    );
    let denied = repository.resolve(ApprovalId::new(21), false)?;
    assert_eq!(
        denied.map(|record| record.status),
        Some(ApprovalStatus::Denied)
    );

    assert!(
        repository.resolve(ApprovalId::new(20), false)?.is_none(),
        "resolving an already-resolved approval must be a no-op"
    );
    assert!(
        repository.resolve(ApprovalId::new(999), true)?.is_none(),
        "resolving an unknown approval must return None"
    );

    let all = repository.find_all()?;
    assert!(
        all.iter()
            .any(|record| record.id == ApprovalId::new(20)
                && record.status == ApprovalStatus::Approved),
        "resolved approval must be visible in find_all with its final status"
    );
    assert!(
        all.iter()
            .any(|record| record.id == ApprovalId::new(21)
                && record.status == ApprovalStatus::Denied),
        "denied approval must be visible in find_all with its final status"
    );
    Ok(())
}

/// The complete shared [`ApprovalRepository`] suite.
pub fn assert_approval_repository_contract(
    repository: &dyn ApprovalRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_approval_repository_round_trip(repository)?;
    assert_approval_repository_pending(repository)?;
    assert_approval_repository_resolution(repository)?;
    Ok(())
}
