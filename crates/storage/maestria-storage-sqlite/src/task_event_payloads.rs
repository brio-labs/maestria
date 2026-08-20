use super::event_payloads::{FamilyDecodeError, StoredApprovalOutcome, StoredEventPayload};
use maestria_domain::{
    ApprovalId, ApprovalOutcome, ArtifactId, DomainEvent, EvidenceId, TaskId, TaskPriority,
    TaskStatus, ValidationReportId,
};
use serde::{Deserialize, Serialize};

crate::stored_enum! {
    /// Stored task priority payload.
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StoredTaskPriority <=> TaskPriority {
        Low,
        Normal,
        High,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredTaskStatus {
    Draft,
    Open,
    Active,
    Validating,
    Blocked,
    CompletedVerified { validation_report_id: u64 },
    CompletedWithWarnings { validation_report_id: u64 },
    Failed,
    Cancelled,
}

impl StoredTaskStatus {
    pub(crate) fn from_domain(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Draft => Self::Draft,
            TaskStatus::Open => Self::Open,
            TaskStatus::Active => Self::Active,
            TaskStatus::Validating => Self::Validating,
            TaskStatus::Blocked => Self::Blocked,
            TaskStatus::CompletedVerified {
                validation_report_id,
            } => Self::CompletedVerified {
                validation_report_id: validation_report_id.value(),
            },
            TaskStatus::CompletedWithWarnings {
                validation_report_id,
            } => Self::CompletedWithWarnings {
                validation_report_id: validation_report_id.value(),
            },
            TaskStatus::Failed => Self::Failed,
            TaskStatus::Cancelled => Self::Cancelled,
        }
    }

    pub(crate) fn into_domain(self) -> TaskStatus {
        match self {
            Self::Draft => TaskStatus::Draft,
            Self::Open => TaskStatus::Open,
            Self::Active => TaskStatus::Active,
            Self::Validating => TaskStatus::Validating,
            Self::Blocked => TaskStatus::Blocked,
            Self::CompletedVerified {
                validation_report_id,
            } => TaskStatus::CompletedVerified {
                validation_report_id: ValidationReportId::new(validation_report_id),
            },
            Self::CompletedWithWarnings {
                validation_report_id,
            } => TaskStatus::CompletedWithWarnings {
                validation_report_id: ValidationReportId::new(validation_report_id),
            },
            Self::Failed => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }
}

impl StoredEventPayload {
    pub(crate) fn try_from_domain_task(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => Some(Self::TaskOpened {
                task_id: task_id.value(),
                title: title.clone(),
                priority: StoredTaskPriority::from_domain(*priority),
                artifact_id: artifact_id.map(|id| id.value()),
            }),
            DomainEvent::TaskStatusChanged { task_id, from, to } => Some(Self::TaskStatusChanged {
                task_id: task_id.value(),
                from: StoredTaskStatus::from_domain(*from),
                to: StoredTaskStatus::from_domain(*to),
            }),
            DomainEvent::TaskCompletionRecorded { task_id, status } => {
                Some(Self::TaskCompletionRecorded {
                    task_id: task_id.value(),
                    status: StoredTaskStatus::from_domain(*status),
                })
            }
            DomainEvent::TaskEvidenceLinked {
                task_id,
                evidence_id,
            } => Some(Self::TaskEvidenceLinked {
                task_id: task_id.value(),
                evidence_id: evidence_id.value(),
            }),
            DomainEvent::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Some(Self::HarnessRunCompleted {
                task_id: task_id.map(|id| id.value()),
                command: command.clone(),
                exit_code: *exit_code,
            }),
            DomainEvent::ApprovalRecorded {
                approval_id,
                outcome,
            } => Some(Self::ApprovalRecorded {
                approval_id: approval_id.value(),
                outcome: stored_approval_outcome(*outcome),
            }),
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Some(Self::ValidationReportCreated {
                report_id: report_id.value(),
                task_id: task_id.map(|id| id.value()),
                passed: *passed,
                warnings: warnings.clone(),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_task(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            Self::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => Ok(DomainEvent::TaskOpened {
                task_id: TaskId::new(task_id),
                title,
                priority: priority
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
                artifact_id: artifact_id.map(ArtifactId::new),
            }),
            Self::TaskStatusChanged { task_id, from, to } => Ok(DomainEvent::TaskStatusChanged {
                task_id: TaskId::new(task_id),
                from: from.into_domain(),
                to: to.into_domain(),
            }),
            Self::TaskCompletionRecorded { task_id, status } => {
                Ok(DomainEvent::TaskCompletionRecorded {
                    task_id: TaskId::new(task_id),
                    status: status.into_domain(),
                })
            }
            Self::TaskEvidenceLinked {
                task_id,
                evidence_id,
            } => Ok(DomainEvent::TaskEvidenceLinked {
                task_id: TaskId::new(task_id),
                evidence_id: EvidenceId::new(evidence_id),
            }),
            Self::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Ok(DomainEvent::HarnessRunCompleted {
                task_id: task_id.map(TaskId::new),
                command,
                exit_code,
            }),
            Self::ApprovalRecorded {
                approval_id,
                outcome,
            } => Ok(DomainEvent::ApprovalRecorded {
                approval_id: ApprovalId::new(approval_id),
                outcome: domain_approval_outcome(outcome),
            }),
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Ok(DomainEvent::ValidationReportCreated {
                report_id: ValidationReportId::new(report_id),
                task_id: task_id.map(TaskId::new),
                passed,
                warnings,
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    pub(crate) fn try_kind_task(&self) -> Option<&'static str> {
        match self {
            Self::TaskOpened { .. } => Some("task_opened"),
            Self::TaskStatusChanged { .. } => Some("task_status_changed"),
            Self::TaskCompletionRecorded { .. } => Some("task_completion_recorded"),
            Self::TaskEvidenceLinked { .. } => Some("task_evidence_linked"),
            Self::HarnessRunCompleted { .. } => Some("harness_run_completed"),
            Self::ApprovalRecorded { .. } => Some("approval_recorded"),
            Self::ValidationReportCreated { .. } => Some("validation_report_created"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_task(&self) -> Option<u64> {
        match self {
            Self::TaskOpened {
                artifact_id: Some(artifact_id),
                ..
            } => Some(*artifact_id),
            _ => None,
        }
    }
}

fn stored_approval_outcome(outcome: ApprovalOutcome) -> StoredApprovalOutcome {
    match outcome {
        ApprovalOutcome::Acknowledged { task_id, approved } => {
            StoredApprovalOutcome::Acknowledged {
                task_id: task_id.map(|id| id.value()),
                approved,
            }
        }
        ApprovalOutcome::TaskTransition {
            task_id,
            approved,
            from_status,
            to_status,
        } => StoredApprovalOutcome::TaskTransition {
            task_id: task_id.value(),
            approved,
            from_status: StoredTaskStatus::from_domain(from_status),
            to_status: StoredTaskStatus::from_domain(to_status),
        },
    }
}

fn domain_approval_outcome(outcome: StoredApprovalOutcome) -> ApprovalOutcome {
    match outcome {
        StoredApprovalOutcome::Acknowledged { task_id, approved } => {
            ApprovalOutcome::Acknowledged {
                task_id: task_id.map(TaskId::new),
                approved,
            }
        }
        StoredApprovalOutcome::TaskTransition {
            task_id,
            approved,
            from_status,
            to_status,
        } => ApprovalOutcome::TaskTransition {
            task_id: TaskId::new(task_id),
            approved,
            from_status: from_status.into_domain(),
            to_status: to_status.into_domain(),
        },
    }
}
