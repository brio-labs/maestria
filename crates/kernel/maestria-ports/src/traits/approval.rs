#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub id: maestria_domain::ApprovalId,
    pub task_id: Option<maestria_domain::TaskId>,
    pub effect_kind: String,
    pub risk_level: ApprovalRiskLevel,
    pub capability: String,
    pub scope_id: maestria_domain::ScopeId,
    pub tick: maestria_domain::LogicalTick,
    pub status: ApprovalStatus,
}

impl ApprovalRecord {
    /// Map a stored approval record to the domain decision both the CLI and the
    /// daemon API submit when resolving it (R28: one owner of approval
    /// resolution semantics). Model-agent approvals are audit acknowledgements
    /// that never transition the linked task; task-activation approvals resolve
    /// (transition) the linked task.
    pub fn to_decision(&self, approved: bool) -> maestria_domain::ApprovalDecision {
        if self.effect_kind == "model_agent_harness" {
            maestria_domain::ApprovalDecision::Acknowledge {
                approval_id: self.id,
                task_id: self.task_id,
                approved,
            }
        } else {
            match self.task_id {
                Some(task_id) => maestria_domain::ApprovalDecision::Resolve {
                    approval_id: self.id,
                    task_id,
                    approved,
                },
                None => maestria_domain::ApprovalDecision::Acknowledge {
                    approval_id: self.id,
                    task_id: None,
                    approved,
                },
            }
        }
    }
}

/// Repository for durable approval requests, independent of governance crate.
pub trait ApprovalRepository: Send + Sync {
    fn save(&self, record: &ApprovalRecord) -> Result<(), crate::PortError>;
    fn find_pending(&self) -> Result<Vec<ApprovalRecord>, crate::PortError>;
    /// Return every approval record, including terminal records, for restart recovery.
    fn find_all(&self) -> Result<Vec<ApprovalRecord>, crate::PortError>;
    fn find_by_id(
        &self,
        id: maestria_domain::ApprovalId,
    ) -> Result<Option<ApprovalRecord>, crate::PortError>;
    fn resolve(
        &self,
        id: maestria_domain::ApprovalId,
        approved: bool,
    ) -> Result<Option<ApprovalRecord>, crate::PortError>;
    fn find_by_task_id(
        &self,
        task_id: maestria_domain::TaskId,
    ) -> Result<Vec<ApprovalRecord>, crate::PortError>;
}
