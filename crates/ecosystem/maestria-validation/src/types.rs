use std::collections::BTreeMap;

use maestria_domain::{
    Claim, ClaimId, Evidence, EvidenceId, MemoryCandidate, MemoryCandidateId, Task, TaskId,
    ValidationReportId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub id: ValidationReportId,
    pub task_id: Option<TaskId>,
    pub checks: Vec<ValidationCheck>,
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCheck {
    pub name: String,
    pub passed: bool,
    pub severity: Severity,
    pub message: String,
}

pub trait Validator: Send + Sync {
    fn name(&self) -> &str;
    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck;
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationContext<'a> {
    pub task: Option<&'a Task>,
    pub claims: &'a BTreeMap<ClaimId, Claim>,
    pub evidences: &'a BTreeMap<EvidenceId, Evidence>,
    pub memory_candidates: &'a BTreeMap<MemoryCandidateId, MemoryCandidate>,
    pub harness_exit_code: Option<i32>,
}
