//! Approval outcome values carried by `DomainEvent::ApprovalResolved`.

use crate::types::*;

/// Outcome recorded by an `ApprovalRecorded` event.
///
/// `Acknowledged` records an operator decision without a task transition
/// (model-agent approvals); the task linkage is audit metadata only.
/// `TaskTransition` records a decision that transitioned a task; the
/// transition is fully specified, so the old correlated `approved` flag plus
/// `Option` status pair is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Acknowledged {
        task_id: Option<TaskId>,
        approved: bool,
    },
    TaskTransition {
        task_id: TaskId,
        approved: bool,
        from_status: TaskStatus,
        to_status: TaskStatus,
    },
}

impl ApprovalOutcome {
    #[must_use]
    pub const fn approved(self) -> bool {
        match self {
            Self::Acknowledged { approved, .. } | Self::TaskTransition { approved, .. } => approved,
        }
    }

    #[must_use]
    pub const fn task_id(self) -> Option<TaskId> {
        match self {
            Self::Acknowledged { task_id, .. } => task_id,
            Self::TaskTransition { task_id, .. } => Some(task_id),
        }
    }
}
