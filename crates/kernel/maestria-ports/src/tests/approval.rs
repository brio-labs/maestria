use crate::{ApprovalRecord, ApprovalRiskLevel, ApprovalStatus};
use maestria_domain::{ApprovalDecision, ApprovalId, LogicalTick, ScopeId, TaskId};

fn record(effect_kind: &str, task_id: Option<u64>) -> ApprovalRecord {
    ApprovalRecord {
        id: ApprovalId::new(1),
        task_id: task_id.map(TaskId::new),
        effect_kind: effect_kind.to_string(),
        risk_level: ApprovalRiskLevel::High,
        capability: String::new(),
        scope_id: ScopeId::new(1),
        tick: LogicalTick::new(1),
        status: ApprovalStatus::Pending,
    }
}

#[test]
fn task_activation_approval_resolves_the_linked_task() {
    let decision = record("task_activation", Some(7)).to_decision(true);
    assert_eq!(
        decision,
        ApprovalDecision::Resolve {
            approval_id: ApprovalId::new(1),
            task_id: TaskId::new(7),
            approved: true,
        }
    );
}

#[test]
fn task_activation_approval_without_task_acknowledges() {
    let decision = record("task_activation", None).to_decision(false);
    assert_eq!(
        decision,
        ApprovalDecision::Acknowledge {
            approval_id: ApprovalId::new(1),
            task_id: None,
            approved: false,
        }
    );
}

#[test]
fn model_agent_approval_always_acknowledges_even_with_task_link() {
    let decision = record("model_agent_harness", Some(7)).to_decision(true);
    assert_eq!(
        decision,
        ApprovalDecision::Acknowledge {
            approval_id: ApprovalId::new(1),
            task_id: Some(TaskId::new(7)),
            approved: true,
        }
    );
}
