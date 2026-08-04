use crate::ids::TaskId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReportRecord {
    pub task_id: Option<TaskId>,
    pub passed: bool,
    pub warnings: Vec<String>,
}
