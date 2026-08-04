use std::collections::BTreeSet;

use crate::ids::{ArtifactId, EvidenceId, TaskId};
use crate::task_status::TaskStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub artifact_ids: BTreeSet<ArtifactId>,
    pub evidence_ids: BTreeSet<EvidenceId>,
}

impl Task {
    pub(crate) fn new(id: TaskId, title: String, priority: TaskPriority) -> Self {
        Self {
            id,
            title,
            priority,
            status: TaskStatus::Draft,
            artifact_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        }
    }
}
