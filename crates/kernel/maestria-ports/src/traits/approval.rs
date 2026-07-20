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
    pub task_id: maestria_domain::TaskId,
    pub effect_kind: String,
    pub risk_level: ApprovalRiskLevel,
    pub capability: String,
    pub scope_id: maestria_domain::ScopeId,
    pub tick: maestria_domain::LogicalTick,
    pub status: ApprovalStatus,
}

/// Repository for durable approval requests, independent of governance crate.
pub trait ApprovalRepository: Send + Sync {
    fn save(&self, record: &ApprovalRecord) -> Result<(), crate::PortError>;
    fn find_pending(&self) -> Result<Vec<ApprovalRecord>, crate::PortError>;
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
