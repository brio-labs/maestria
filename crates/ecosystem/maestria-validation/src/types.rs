use std::collections::BTreeMap;

use maestria_domain::{
    Artifact, ArtifactId, Claim, ClaimId, Evidence, EvidenceId, MemoryCandidate, MemoryCandidateId,
    Task, TaskId, ValidationReportId,
};

#[path = "search_context.rs"]
mod search_context;
pub use search_context::SearchValidationContext;

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

#[derive(Debug, Clone)]
pub struct ValidationContext<'a> {
    pub task: Option<&'a Task>,
    pub artifacts: &'a BTreeMap<ArtifactId, Artifact>,
    pub claims: &'a BTreeMap<ClaimId, Claim>,
    pub evidences: &'a BTreeMap<EvidenceId, Evidence>,
    pub memory_candidates: &'a BTreeMap<MemoryCandidateId, MemoryCandidate>,
    pub harness_exit_code: Option<i32>,
    pub search: Option<SearchValidationContext<'a>>,
}

pub(crate) fn passed_check(name: &str, message: impl Into<String>) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: true,
        severity: Severity::Error,
        message: message.into(),
    }
}

pub(crate) fn failed_check(name: &str, message: impl Into<String>) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: false,
        severity: Severity::Error,
        message: message.into(),
    }
}

pub(crate) fn count_missing_evidence(
    ids: impl Iterator<Item = EvidenceId>,
    evidences: &BTreeMap<EvidenceId, Evidence>,
) -> usize {
    ids.filter(|id| !evidences.contains_key(id)).count()
}
